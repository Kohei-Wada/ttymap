use std::io;
use std::sync::Arc;
use std::time::Duration;

use crossterm::event::{self, Event, KeyModifiers};
use log::{debug, info};
use ratatui::DefaultTerminal;

use crate::config::Config;
use crate::core::{Core, CoreOptions};
use crate::keyboard::KeyboardHandler;
use crate::keymap::KeyMap;
use crate::mouse::MouseHandler;
use crate::render::pipeline::RenderPipeline;
use crate::render::thread::{RenderHandle, RenderResult};
use crate::shared::nominatim::NominatimClient;
use crate::styler::Styler;
use crate::ui::UiState;
use crate::ui::widget::Widget;

/// What a key or mouse event just changed. Drives how the main loop
/// reacts: a widget-only change redraws immediately (the map frame is
/// unchanged); a map change only requests a new render — the main
/// loop will redraw when a fresh frame arrives, avoiding a
/// stale-frame draw followed by a second fresh-frame draw.
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum InputEffect {
    None,
    Widget,
    Map,
}

pub struct App {
    core: Core,
    keyboard: KeyboardHandler,
    render_handle: RenderHandle,
    ui: UiState,
    mouse: MouseHandler,
}

impl App {
    pub fn new(config: Config) -> Self {
        let theme_id = crate::palette::ThemeId::from_name(&config.style);
        let styler = Arc::new(Styler::new(theme_id));

        let (cols, rows) = crossterm::terminal::size().unwrap_or((80, 24));
        let (width, height) = crate::render::canvas_size(cols, rows);

        info!(
            "terminal size: {}x{}, canvas: {}x{}",
            cols, rows, width, height
        );

        let palette = styler.palette();
        let nominatim = Arc::new(NominatimClient::new());
        let (tile_cache, attribution) = build_tile_cache(&config);
        let mut ui = UiState::new(
            palette,
            &config.language,
            config.wiki_limit,
            nominatim,
            attribution,
        );
        let pipeline =
            RenderPipeline::new(tile_cache, styler, config.language.clone(), width, height);

        let keymap = KeyMap::with_overrides(&config.keymap);
        let core = Core::new(
            CoreOptions {
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
        ui.help.build(keyboard.keymap());

        App {
            core,
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

        'main_loop: while self.core.is_running() {
            // Drain completed render frames into UiState.
            while let Ok(RenderResult::Frame(frame)) = self.render_handle.result_rx.try_recv() {
                self.ui.map_frame = Some(frame);
            }

            // Poll widgets with background fetches.
            self.ui.search.poll();
            self.ui.info.poll();
            self.ui.wiki.poll();

            self.draw_terminal(&mut terminal)?;

            // Drain the whole input queue in one pass so a burst of key-repeat
            // or mouse-drag events produces one redraw next iteration, not one
            // draw per event. First poll blocks up to 4 ms so render-thread
            // frame arrivals (which don't wake the event poll) show up within
            // ~4 ms at worst; subsequent polls use zero timeout.
            let mut poll_timeout = Duration::from_millis(4);
            while event::poll(poll_timeout)? {
                poll_timeout = Duration::from_millis(0);
                match event::read()? {
                    Event::Key(key_event) => {
                        if key_event.modifiers.contains(KeyModifiers::CONTROL)
                            && key_event.code == crossterm::event::KeyCode::Char('c')
                        {
                            info!("Ctrl-C received, quitting");
                            self.core.stop();
                            break 'main_loop;
                        }

                        debug!("key event: {:?}", key_event.code);
                        if let InputEffect::Map = self.keyboard.handle(
                            key_event.code,
                            key_event.modifiers,
                            &mut self.core,
                            &mut self.ui,
                        ) && self.core.is_running()
                        {
                            self.request_draw();
                        }
                    }
                    Event::Resize(cols, rows) => {
                        info!("resize: {}x{}", cols, rows);
                        self.core.resize(cols, rows);
                        self.render_handle
                            .request_resize(self.core.width(), self.core.height());
                        self.request_draw();
                    }
                    Event::Mouse(mouse) => {
                        if let InputEffect::Map =
                            self.mouse.handle(mouse, &mut self.core, &mut self.ui)
                        {
                            self.request_draw();
                        }
                    }
                    _ => {}
                }
            }
        }

        info!("event loop ended, shutting down render thread");
        self.render_handle.shutdown();
        crossterm::execute!(io::stdout(), crossterm::event::DisableMouseCapture)?;
        ratatui::restore();
        info!("terminal restored, exiting");

        Ok(())
    }

    fn request_draw(&mut self) {
        let state = self.core.render_request();
        self.render_handle.request_draw(state);

        // Notify passive widgets that the map recentered. They decide
        // internally whether to act (e.g., place throttles to 5s).
        // Wiki is intentionally not notified — Google-Maps-style, the
        // article list stays pinned to the query that produced it.
        if !self.ui.search.is_active() {
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
fn build_tile_cache(config: &Config) -> (crate::tile::TileCache, Option<String>) {
    use crate::tile::fetch::TileClient;
    let (tx, rx) = std::sync::mpsc::channel();
    let client = crate::tile::fetch::MapsciiTileClient::new(tx);
    let attribution = {
        let s = client.attribution();
        (!s.is_empty()).then(|| s.to_string())
    };
    let boxed: Box<dyn TileClient> = Box::new(client);
    (
        crate::tile::TileCache::new(boxed, rx, config.cache_tiles),
        attribution,
    )
}
