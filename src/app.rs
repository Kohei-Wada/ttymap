use std::io;
use std::sync::Arc;
use std::time::Duration;

use crossterm::event::{self, Event, KeyCode, KeyModifiers};
use log::{debug, info};

use crate::command::{self, Command, DispatchCtx, InputEffect};
use crate::config::Config;
use crate::input::keyboard::KeyboardHandler;
use crate::input::mouse::MouseHandler;
use crate::keymap::KeyMap;
use crate::map::render::pipeline::RenderPipeline;
use crate::map::render::thread::RenderHandle;
use crate::map::styler::Styler;
use crate::map::{Action, MapState, MapStateOptions};
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
        self.dispatch(Command::Map(Action::Redraw));

        while self.map.is_running() {
            self.ui.drain_frames(&self.render_handle);

            if let Some(cmd) = self.ui.poll_widgets() {
                info!("plugin async command: {:?}", cmd);
                self.dispatch(cmd);
            }

            self.ui.info.poll();

            terminal.draw(|f| crate::ui::draw(f, &self.ui))?;

            // Drain the whole input queue in one pass. First poll blocks up to
            // 4 ms so render-thread frame arrivals (which don't wake the event
            // poll) show up within ~4 ms at worst; subsequent polls use zero
            // timeout.
            let mut poll_timeout = Duration::from_millis(4);
            while event::poll(poll_timeout)? {
                poll_timeout = Duration::from_millis(0);
                match event::read()? {
                    Event::Key(key_event) => {
                        // Ctrl-C is a host-level safety valve, bypassing the keymap.
                        if key_event.modifiers.contains(KeyModifiers::CONTROL)
                            && key_event.code == KeyCode::Char('c')
                        {
                            info!("Ctrl-C received, quitting");
                            self.dispatch(Command::Map(Action::Quit));
                        } else {
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
                    }
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
        }

        info!("event loop ended");
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
