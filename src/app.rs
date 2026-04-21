use std::io;
use std::sync::Arc;
use std::time::Duration;

use crossterm::event::{self, Event, KeyCode, KeyModifiers};
use log::{debug, info};

use crate::app_command::{self, AppCommand, DispatchCtx, InputEffect};
use crate::background::BackgroundResponder;
use crate::color_palette::ThemeId;
use crate::config::Config;
use crate::focus::FocusManager;
use crate::input::mouse::MouseHandler;
use crate::keymap::KeyMap;
use crate::map::render::pipeline::RenderPipeline;
use crate::map::render::thread::RenderHandle;
use crate::map::styler::Styler;
use crate::map::{Action, MapState, MapStateOptions};
use crate::plugin::PluginRegistry;
use crate::plugin::help::HelpPlugin;
use crate::plugin::here::HerePlugin;
use crate::plugin::search::SearchPlugin;
use crate::plugin::wiki::WikiPlugin;
use crate::shared::nominatim::NominatimClient;
use crate::theme::UiTheme;
use crate::ui::UiState;
use crate::ui::palette::CommandPalette;
use crate::ui::router;

pub struct App {
    map: MapState,
    render_handle: RenderHandle,
    ui: UiState,
    mouse: MouseHandler,
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
        let widgets = build_plugin_registry(&config, nominatim.clone(), &keymap);
        let activations = widgets.activations();
        let background = BackgroundResponder::new(keymap, activations);
        let focus = FocusManager::new(CommandPalette::new(), widgets, background);
        let ui = UiState::new(nominatim, attribution, focus);
        let theme_id = ThemeId::from_name(&config.style);
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
            mouse: MouseHandler::default(),
            theme_id,
            ui_theme,
        }
    }

    pub fn run(&mut self) -> io::Result<()> {
        let mut terminal = ratatui::init();
        crossterm::execute!(io::stdout(), crossterm::event::EnableMouseCapture)?;

        info!("event loop started");
        self.dispatch(AppCommand::Map(Action::Redraw));

        while self.map.is_running() {
            self.ui.drain_frames(&self.render_handle);

            if let Some(cmd) = self.ui.focus.poll_widgets() {
                info!("plugin async command: {:?}", cmd);
                self.dispatch(cmd);
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
                            self.dispatch(AppCommand::Map(Action::Quit));
                        } else {
                            debug!("key event: {:?}", key_event.code);
                            if let Some(cmd) = router::route_key(
                                &mut self.ui.focus,
                                key_event.code,
                                key_event.modifiers,
                                self.map.center(),
                            ) {
                                self.dispatch(cmd);
                            }
                        }
                    }
                    Event::Resize(cols, rows) => {
                        info!("resize: {}x{}", cols, rows);
                        self.dispatch(AppCommand::Resize(cols, rows));
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
    fn dispatch(&mut self, cmd: AppCommand) {
        let effect = {
            let mut ctx = DispatchCtx {
                map: &mut self.map,
                ui: &mut self.ui,
                render_handle: &self.render_handle,
                theme_id: &mut self.theme_id,
                ui_theme: &mut self.ui_theme,
            };
            app_command::dispatch(cmd, &mut ctx)
        };
        if matches!(effect, InputEffect::Map) && self.map.is_running() {
            let viewport = self.map.viewport();
            self.render_handle.request_draw(viewport);
            // Notify passive widgets that the map recentered. They decide
            // internally whether to act (e.g., place throttles to 5s).
            // Wiki is intentionally not notified — Google-Maps-style, the
            // article list stays pinned to the query that produced it.
            if !self.ui.focus.is_modal("search") {
                self.ui.overlay.on_map_moved(viewport.center);
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

/// Composition root for plugins: instantiates the four built-in
/// plugins, lets `HelpPlugin` introspect the others' activation keys,
/// and returns a populated registry. Lives here (not in `UiState::new`)
/// so adding a new plugin doesn't require touching the UI module.
fn build_plugin_registry(
    config: &Config,
    nominatim: Arc<NominatimClient>,
    keymap: &KeyMap,
) -> PluginRegistry {
    let search = SearchPlugin::new(nominatim);
    let mut help = HelpPlugin::new();
    let wiki = WikiPlugin::new(&config.language, config.wiki_limit);
    let here = HerePlugin::new(config.geoip_endpoint.clone(), config.geoip_timeout_ms);

    // Help introspects the other plugins to list their activation
    // keys, so it must build after they're constructed. Palette is a
    // builtin (see `ui::palette`) so help references it directly via
    // a hardcoded line rather than a `Plugin` trait object.
    help.build(keymap, &[&search, &wiki]);

    let mut widgets = PluginRegistry::new();
    // Registration order = dispatch priority for action broadcasts.
    widgets.register(Box::new(search));
    widgets.register(Box::new(help));
    widgets.register(Box::new(wiki));
    widgets.register(Box::new(here));
    widgets
}
