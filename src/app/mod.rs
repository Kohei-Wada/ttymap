//! Application event loop and central message dispatcher.
//!
//! [`App`] is the sole **Receiver** in the GoF Command pattern: every
//! invoker (keymap, palette, plugins, mouse adapter) returns
//! `Vec<AppMsg>` and only `App::dispatch` executes them. The
//! dispatcher is a thin router — each [`AppMsg`] arm either delegates
//! to a method on the domain type that owns the relevant state
//! ([`MapState`] / [`UiState`]) or, for cross-cutting transitions
//! that don't fit a single domain ([`AppMsg::SetTheme`] and
//! [`AppMsg::Resize`]), delegates to a method on `App` itself.
//!
//! This keeps the "what changed?" knowledge local: arms whose effect
//! changed the map frame call [`App::request_map_redraw`] through
//! their delegate, instead of the router having to guess from outside.

pub mod msg;

pub use msg::AppMsg;

use std::io;
use std::sync::Arc;
use std::time::Duration;

use crossterm::event::{self, Event, KeyCode, KeyModifiers};
use log::{debug, info};

use crate::background::BackgroundResponder;
use crate::color_palette::ThemeId;
use crate::config::Config;
use crate::focus::{FocusManager, SurfaceCtx};
use crate::keymap::KeyMap;
use crate::map::render::pipeline::RenderPipeline;
use crate::map::render::thread::RenderHandle;
use crate::map::styler::Styler;
use crate::map::{Action, MapState, MapStateOptions};
use crate::plugin::PluginRegistry;
use crate::plugin::help::HelpPlugin;
use crate::plugin::here::HerePlugin;
use crate::plugin::palette::CommandPalette;
use crate::plugin::search::SearchPlugin;
use crate::plugin::wiki::WikiPlugin;
use crate::shared::nominatim::NominatimClient;
use crate::theme::UiTheme;
use crate::ui::UiState;
use crate::ui::mouse::MouseAdapter;
use crate::ui::router;

pub struct App {
    map: MapState,
    render_handle: RenderHandle,
    ui: UiState,
    mouse: MouseAdapter,
    /// Active theme — single source of truth for the running app.
    /// `ui_theme` is its derived UI-colour cache; the render thread
    /// receives a corresponding `Styler` via message on switch.
    theme_id: ThemeId,
    ui_theme: UiTheme,
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
        let theme_id = ThemeId::from_name(&config.style);
        let widgets = build_plugin_registry(&config, nominatim.clone(), &keymap);
        let activations = widgets.activations();
        let background = BackgroundResponder::new(keymap, activations);
        let focus = FocusManager::new(widgets, background);
        let ui = UiState::new(nominatim, attribution, focus);
        let ui_theme = UiTheme::from_palette(theme_id.palette());
        let styler = Arc::new(Styler::new(theme_id));
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

        App {
            map,
            render_handle,
            ui,
            mouse: MouseAdapter::default(),
            theme_id,
            ui_theme,
        }
    }

    pub fn run(&mut self) -> io::Result<()> {
        let mut terminal = ratatui::init();
        crossterm::execute!(io::stdout(), crossterm::event::EnableMouseCapture)?;

        info!("event loop started");
        self.dispatch(AppMsg::Map(Action::Redraw));

        while self.map.is_running() {
            self.ui.drain_frames(&self.render_handle);

            for msg in self.ui.focus.poll_widgets() {
                info!("plugin async msg: {:?}", msg);
                self.dispatch(msg);
            }

            self.ui.overlay.poll();

            terminal.draw(|f| crate::ui::draw(f, &self.ui, &self.ui_theme))?;

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
                            self.dispatch(AppMsg::Map(Action::Quit));
                        } else {
                            debug!("key event: {:?}", key_event.code);
                            let ctx = SurfaceCtx {
                                center: self.map.center(),
                                theme_id: self.theme_id,
                            };
                            for msg in router::route_key(&mut self.ui.focus, key_event, ctx) {
                                self.dispatch(msg);
                            }
                        }
                    }
                    Event::Resize(cols, rows) => {
                        info!("resize: {}x{}", cols, rows);
                        self.dispatch(AppMsg::Resize(cols, rows));
                    }
                    Event::Mouse(mouse) => {
                        for msg in self.mouse.translate(mouse) {
                            self.dispatch(msg);
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

    /// Apply an `AppMsg` to the app. Thin router: each arm either
    /// delegates to a method on the domain type that owns the
    /// relevant state, or — for cross-cutting transitions — to a
    /// method on `self`. Those delegates request a map redraw via
    /// [`request_map_redraw`](Self::request_map_redraw) when their
    /// effect changed the map frame.
    fn dispatch(&mut self, msg: AppMsg) {
        match msg {
            AppMsg::Map(action) => {
                if self.map.process_action(&action) {
                    self.request_map_redraw();
                }
            }
            AppMsg::Jump(loc) => {
                self.map.jump_to(loc);
                self.request_map_redraw();
            }
            AppMsg::SetTheme(new_id) => self.apply_theme(new_id),
            AppMsg::CursorMoved(col, row) => {
                self.ui.overlay.set_cursor((col, row));
            }
            AppMsg::CycleFocus(forward) => {
                self.ui.focus.cycle(forward);
            }
            AppMsg::Resize(cols, rows) => self.handle_resize(cols, rows),
        }
    }

    /// Cross-cutting: re-derive the UI colour cache and map styler
    /// from the new theme id, notify the render thread, and force a
    /// redraw so the change is visible without waiting for another
    /// map event. The palette's theme-picker entry reads `theme_id`
    /// via `SurfaceCtx` on activation, so no surface-level push.
    fn apply_theme(&mut self, new_id: ThemeId) {
        self.theme_id = new_id;
        let styler = Arc::new(Styler::new(new_id));
        self.ui_theme = UiTheme::from_palette(styler.palette());
        self.render_handle.set_styler(styler);
        self.request_map_redraw();
    }

    /// Cross-cutting: update map viewport + render thread canvas to
    /// the new terminal size, then request a fresh frame.
    fn handle_resize(&mut self, cols: u16, rows: u16) {
        self.map.resize(cols, rows);
        self.render_handle
            .request_resize(self.map.width(), self.map.height());
        self.request_map_redraw();
    }

    /// Request a fresh map frame from the render thread and notify
    /// passive widgets that the map recentered. Called by the
    /// dispatch delegates whose effect changed the map frame. No-op
    /// after shutdown (`map.is_running() == false`).
    ///
    /// Wiki is intentionally not notified — Google-Maps-style, the
    /// article list stays pinned to the query that produced it.
    fn request_map_redraw(&mut self) {
        if !self.map.is_running() {
            return;
        }
        let viewport = self.map.viewport();
        self.render_handle.request_draw(viewport);
        self.ui.overlay.on_map_moved(viewport.center);
    }
}

/// Composition root for the tile subsystem: selects a `TileClient`
/// from config and wires it to a fresh `TileCache`. Backend selection
/// lives here (not in `tile/`) so swapping mapscii for mbtiles/pmtiles
/// stays visible alongside the rest of app wiring. Also snapshots the
/// client's attribution string before boxing, so the UI can display it
/// without needing a live handle to the (moved) client.
pub(crate) fn build_tile_cache(config: &Config) -> (crate::map::tile::TileCache, Option<String>) {
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

/// Composition root for plugins: instantiates the five built-in
/// plugins (`search`, `help`, `wiki`, `here`, `palette`), lets the
/// two introspection-driven ones (`help`, `palette`) walk the others
/// for their entries, and returns a populated registry. Lives here
/// (not in `UiState::new`) so adding a new plugin doesn't require
/// touching the UI module.
fn build_plugin_registry(
    config: &Config,
    nominatim: Arc<NominatimClient>,
    keymap: &KeyMap,
) -> PluginRegistry {
    let search = SearchPlugin::new(nominatim);
    let mut help = HelpPlugin::new();
    let wiki = WikiPlugin::new(&config.language, config.wiki_limit);
    let here = HerePlugin::new(config.geoip_endpoint.clone(), config.geoip_timeout_ms);
    let mut palette = CommandPalette::new();

    // Help and palette both walk sibling plugins to capture their
    // descriptions / activation keys. Both must be built before
    // they're moved into the registry. Help still hardcodes its
    // palette line because palette has no description (it opts out
    // of being listed inside itself / inside help).
    help.build(keymap, &[&search, &wiki]);
    palette.build(keymap, &[&search, &help, &wiki, &here]);

    let mut widgets = PluginRegistry::new();
    // Registration order = paint order in `ui::draw` (later entries
    // draw on top). Palette goes last so its popup overlays any
    // simultaneously visible plugin panel.
    widgets.register(Box::new(search));
    widgets.register(Box::new(help));
    widgets.register(Box::new(wiki));
    widgets.register(Box::new(here));
    widgets.register(Box::new(palette));
    widgets
}
