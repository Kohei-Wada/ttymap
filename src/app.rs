use std::io;
use std::sync::Arc;
use std::time::Duration;

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use log::{debug, info};
use ratatui::DefaultTerminal;

use crate::command::{self, Command, DispatchCtx, InputEffect};
use crate::config::Config;
use crate::input::keyboard::KeyboardHandler;
use crate::input::mouse::MouseHandler;
use crate::keymap::KeyMap;
use crate::map::render::pipeline::RenderPipeline;
use crate::map::render::thread::{RenderHandle, RenderResult};
use crate::map::styler::Styler;
use crate::map::{MapState, MapStateOptions};
use crate::shared::nominatim::NominatimClient;
use crate::ui::UiState;

pub struct App {
    map: MapState,
    keyboard: KeyboardHandler,
    render_handle: RenderHandle,
    ui: UiState,
    mouse: MouseHandler,
}

impl App {
    pub fn new(config: Config) -> Self {
        let (cols, rows) = crossterm::terminal::size().unwrap_or((80, 24));
        let (width, height) = crate::map::render::canvas_size(cols, rows);

        info!(
            "terminal size: {}x{}, canvas: {}x{}",
            cols, rows, width, height
        );

        let nominatim = Arc::new(NominatimClient::new());
        let (tile_cache, attribution) = build_tile_cache(&config);
        let keymap = KeyMap::with_overrides(&config.keymap);
        let ui = UiState::new(&config, nominatim, attribution, &keymap);
        let styler = Arc::new(Styler::new(ui.theme_id));
        let pipeline =
            RenderPipeline::new(tile_cache, styler, config.language.clone(), width, height);
        let map = MapState::new(
            MapStateOptions {
                initial_lon: config.initial_lon,
                initial_lat: config.initial_lat,
                initial_zoom: config.initial_zoom,
                zoom_step: config.zoom_step,
                max_zoom: config.max_zoom,
            },
            width,
            height,
        );
        let render_handle = RenderHandle::spawn(pipeline);
        let keyboard = KeyboardHandler::new(keymap);

        App {
            map,
            keyboard,
            render_handle,
            ui,
            mouse: MouseHandler::default(),
        }
    }

    pub fn run(&mut self) -> io::Result<()> {
        let mut terminal = ratatui::init();
        crossterm::execute!(io::stdout(), crossterm::event::EnableMouseCapture)?;

        info!("event loop started");
        self.request_draw();

        while self.map.is_running() {
            self.drain_render_frames();
            self.poll_widgets();
            self.ui.info.poll();
            self.draw_terminal(&mut terminal)?;
            self.drain_input_events()?;
        }

        info!("event loop ended, shutting down render thread");
        self.render_handle.shutdown();
        crossterm::execute!(io::stdout(), crossterm::event::DisableMouseCapture)?;
        ratatui::restore();
        info!("terminal restored, exiting");

        Ok(())
    }

    /// Run a command through the controller, then request a new map
    /// frame if the command changed map state. Single entry point for
    /// every `dispatch(...)` call site so the ctx bundle and
    /// post-dispatch redraw rule live in exactly one place.
    fn dispatch(&mut self, cmd: Command) {
        let effect = {
            let mut ctx = DispatchCtx {
                map: &mut self.map,
                ui: &mut self.ui,
                render_handle: &self.render_handle,
                keymap: self.keyboard.keymap(),
            };
            command::dispatch(cmd, &mut ctx)
        };
        if matches!(effect, InputEffect::Map) && self.map.is_running() {
            self.request_draw();
        }
    }

    /// Pick up any `MapFrame` the render thread has produced since the
    /// last tick and hand it to the UI for drawing.
    fn drain_render_frames(&mut self) {
        while let Ok(RenderResult::Frame(frame)) = self.render_handle.result_rx.try_recv() {
            self.ui.map_frame = Some(frame);
        }
    }

    /// Poll every plugin for background work and dispatch any deferred
    /// `Command` one emitted (e.g. the `here` plugin surfacing a
    /// geoip-resolved `Command::Jump`). Only the latest pending
    /// command is applied per tick.
    fn poll_widgets(&mut self) {
        let mut async_cmd: Option<Command> = None;
        for w in self.ui.widgets.iter_mut() {
            w.poll();
            if let Some(cmd) = w.pending_command() {
                async_cmd = Some(cmd);
            }
        }
        if let Some(cmd) = async_cmd {
            info!("plugin async command: {:?}", cmd);
            self.dispatch(cmd);
        }
    }

    /// Drain the whole input queue in one pass so a burst of key-repeat
    /// or mouse-drag events produces one redraw next iteration, not
    /// one draw per event. First poll blocks up to 4 ms so render-
    /// thread frame arrivals (which don't wake the event poll) show
    /// up within ~4 ms at worst; subsequent polls use zero timeout.
    fn drain_input_events(&mut self) -> io::Result<()> {
        let mut poll_timeout = Duration::from_millis(4);
        while event::poll(poll_timeout)? {
            poll_timeout = Duration::from_millis(0);
            match event::read()? {
                Event::Key(key_event) => self.handle_key_event(key_event),
                Event::Resize(cols, rows) => {
                    info!("resize: {}x{}", cols, rows);
                    self.dispatch(Command::Resize(cols, rows));
                }
                Event::Mouse(mouse) => {
                    if let Some(cmd) = self.mouse.handle(mouse, &mut self.ui) {
                        self.dispatch(cmd);
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// Translate a key event into an optional `Command` via the
    /// keyboard handler and dispatch it. Ctrl-C is handled as a
    /// host-level safety valve, bypassing the keymap.
    fn handle_key_event(&mut self, key_event: KeyEvent) {
        if key_event.modifiers.contains(KeyModifiers::CONTROL)
            && key_event.code == KeyCode::Char('c')
        {
            info!("Ctrl-C received, quitting");
            self.map.stop();
            return;
        }
        debug!("key event: {:?}", key_event.code);
        if let Some(cmd) = self.keyboard.handle(
            key_event.code,
            key_event.modifiers,
            &mut self.ui,
            self.map.center(),
        ) {
            self.dispatch(cmd);
        }
    }

    fn request_draw(&mut self) {
        let state = self.map.render_request();
        self.render_handle.request_draw(state);

        // Notify passive widgets that the map recentered. They decide
        // internally whether to act (e.g., place throttles to 5s).
        // Wiki is intentionally not notified — Google-Maps-style, the
        // article list stays pinned to the query that produced it.
        if !self.ui.focus.is_plugin("search") {
            self.ui.info.on_map_moved(state.center);
        }
    }

    fn draw_terminal(&mut self, terminal: &mut DefaultTerminal) -> io::Result<()> {
        // Coords and scale bar pull directly from the MapFrame inside
        // their overlays, so app.rs no longer pushes derived strings.
        terminal.draw(|f| {
            crate::ui::draw(f, &self.ui);
        })?;
        Ok(())
    }
}

/// Composition root for the tile subsystem: selects a `TileClient`
/// from config and wires it to a fresh `TileCache`. Backend selection
/// lives here (not in `tile/`) so swapping mapscii for mbtiles/pmtiles
/// stays visible alongside the rest of app wiring. Also snapshots the
/// client's attribution string before boxing, so the UI can display it
/// without needing a live handle to the (moved) client.
fn build_tile_cache(config: &Config) -> (crate::map::tile::TileCache, Option<String>) {
    use crate::map::tile::fetch::TileClient;
    let (tx, rx) = std::sync::mpsc::channel();
    let client = crate::map::tile::fetch::MapsciiTileClient::new(tx);
    let attribution = {
        let s = client.attribution();
        (!s.is_empty()).then(|| s.to_string())
    };
    let boxed: Box<dyn TileClient> = Box::new(client);
    (
        crate::map::tile::TileCache::new(boxed, rx, config.cache_tiles),
        attribution,
    )
}
