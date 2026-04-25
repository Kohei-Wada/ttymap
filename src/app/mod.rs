//! Application event loop and central message dispatcher.
//!
//! [`App`] is the sole **Receiver** in the GoF Command pattern: every
//! invoker (keymap, palette, plugins, mouse adapter) returns
//! `Vec<AppMsg>` and only `App::dispatch` executes them.
//!
//! Focus/modal state lives on [`Compositor`] — a stack of
//! [`Component`]s that replaced the old `FocusManager` + `Plugin`
//! trilogy. Components render map overlays through
//! `Component::paint_on_map`. Headless async jobs (here plugin's
//! geoip) live in a [`Task`] list. Neither channel carries a
//! concrete plugin type — they contain only `Box<dyn Component>` /
//! `Box<dyn Task>`, populated by each plugin's `register` function
//! at composition time.

mod mouse;
pub mod msg;

pub use msg::AppMsg;

use std::io;
use std::sync::Arc;

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use log::{debug, info};

use crate::compositor::{BaseLayer, Compositor, Context, Registrar, Task};
use crate::config::Config;
use crate::keymap::KeyMap;
use crate::map::render::frame::MapFrame;
use crate::map::render::pipeline::RenderPipeline;
use crate::map::render::thread::RenderHandle;
use crate::map::styler::Styler;
use crate::map::{Action, MapState, MapStateOptions};
use crate::plugin_api::nominatim::NominatimClient;
use crate::theme::ThemeId;
use crate::theme::UiTheme;
use mouse::MouseAdapter;

pub struct App {
    map: MapState,
    render_handle: RenderHandle,
    /// Latest rendered map snapshot drained from the render thread.
    /// `None` until the first frame arrives. Owned here directly —
    /// no UiState wrapper now that built-in chrome lives in plugins.
    map_frame: Option<MapFrame>,
    mouse: MouseAdapter,
    compositor: Compositor,
    tasks: Vec<Box<dyn Task>>,
    theme_id: ThemeId,
    ui_theme: UiTheme,
    /// Latest mouse cursor position in absolute terminal cells.
    /// `None` until the first mouse event arrives (or always, on
    /// terminals without mouse support). Surfaced to components via
    /// [`Context`](crate::compositor::Context) so plugins can build
    /// cursor-aware overlays.
    cursor: Option<(u16, u16)>,
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
        let theme_id = ThemeId::from_name(&config.render.style);
        let registrar = build_registrar(&config, nominatim, attribution, &keymap);
        let ui_theme = UiTheme::from_palette(theme_id.palette());
        let styler = Arc::new(Styler::new(theme_id));
        let pipeline = RenderPipeline::new(
            tile_cache,
            styler,
            config.render.language.clone(),
            width,
            height,
        );
        let map = MapState::new(
            MapStateOptions {
                initial_lon: config.map.lon,
                initial_lat: config.map.lat,
                initial_zoom: config.map.zoom,
                zoom_step: config.map.zoom_step,
                max_zoom: config.map.max_zoom,
            },
            width,
            height,
        );
        let render_handle = RenderHandle::spawn(pipeline);

        // Compositor bootstraps with the BaseLayer (keymap +
        // activation dispatch) at index 0. Every subsequent modal is
        // pushed on top.
        let mut compositor = Compositor::new();
        compositor.push(Box::new(BaseLayer::new(keymap, registrar.activations)));
        // Drain always-on overlay factories from the registrar and
        // install each overlay. The seed Context uses the same
        // values App::context produces; cursor starts as None.
        let overlay_ctx = Context {
            center: map.center(),
            theme_id,
            cursor: None,
        };
        for factory in registrar.overlays {
            compositor.add_overlay(factory(&overlay_ctx));
        }

        App {
            map,
            render_handle,
            map_frame: None,
            mouse: MouseAdapter::default(),
            compositor,
            tasks: registrar.tasks,
            theme_id,
            ui_theme,
            cursor: None,
        }
    }

    pub fn run(&mut self) -> io::Result<()> {
        use std::time::Duration;

        let mut terminal = ratatui::init();
        crossterm::execute!(io::stdout(), crossterm::event::EnableMouseCapture)?;

        info!("event loop started");
        self.dispatch(AppMsg::Map(Action::Redraw));

        while self.map.is_running() {
            // Drain every frame the render thread has produced; the
            // last one wins (stale ones are discarded).
            while let Some(frame) = self.render_handle.try_recv_frame() {
                self.map_frame = Some(frame);
            }

            // Drain per-tick messages: compositor components first,
            // then tasks. Both borrow &mut self transitively through
            // dispatch, so collect into a Vec first.
            let ctx = self.context();
            let compositor_msgs = self.compositor.poll(&ctx);
            for msg in compositor_msgs {
                self.dispatch(msg);
            }
            let task_msgs: Vec<AppMsg> = self.tasks.iter_mut().flat_map(|t| t.poll()).collect();
            for msg in task_msgs {
                self.dispatch(msg);
            }

            let ctx = self.context();
            terminal.draw(|f| {
                crate::ui::draw(
                    f,
                    self.map_frame.as_ref(),
                    &self.compositor,
                    &self.ui_theme,
                    &ctx,
                )
            })?;

            let mut poll_timeout = Duration::from_millis(4);
            while event::poll(poll_timeout)? {
                poll_timeout = Duration::from_millis(0);
                match event::read()? {
                    Event::Key(key_event) => {
                        if key_event.modifiers.contains(KeyModifiers::CONTROL)
                            && key_event.code == KeyCode::Char('c')
                        {
                            info!("Ctrl-C received, quitting");
                            self.dispatch(AppMsg::Map(Action::Quit));
                        } else {
                            debug!("key event: {:?}", key_event.code);
                            self.handle_key(key_event);
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

    fn handle_key(&mut self, key: KeyEvent) {
        let ctx = self.context();
        let msgs = self.compositor.handle_event(key, &ctx);
        for msg in msgs {
            self.dispatch(msg);
        }
    }

    fn context(&self) -> Context {
        Context {
            center: self.map.center(),
            theme_id: self.theme_id,
            cursor: self.cursor,
        }
    }

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
                self.cursor = Some((col, row));
            }
            AppMsg::CycleFocus(forward) => {
                self.compositor.cycle(forward);
            }
            AppMsg::Resize(cols, rows) => self.handle_resize(cols, rows),
            AppMsg::ExportFrame => self.export_current_frame(),
        }
    }

    fn export_current_frame(&self) {
        let Some(frame) = self.map_frame.as_ref() else {
            log::warn!("export: no frame to write yet");
            return;
        };
        let Some(dirs) = directories::ProjectDirs::from("", "", "ttymap") else {
            log::warn!("export: no ProjectDirs available");
            return;
        };
        let dir = dirs.data_dir().join("exports");
        if let Err(e) = std::fs::create_dir_all(&dir) {
            log::warn!("export: mkdir {} failed: {e}", dir.display());
            return;
        }
        let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
        let name = format!(
            "ttymap-z{}-{:.4}-{:.4}-{}.ans",
            frame.zoom.floor() as i32,
            frame.center.lat,
            frame.center.lon,
            stamp
        );
        let path = dir.join(&name);
        match std::fs::write(&path, frame.to_ansi()) {
            Ok(()) => log::info!("export: wrote {}", path.display()),
            Err(e) => log::warn!("export: write {} failed: {e}", path.display()),
        }
    }

    fn apply_theme(&mut self, new_id: ThemeId) {
        self.theme_id = new_id;
        let styler = Arc::new(Styler::new(new_id));
        self.ui_theme = UiTheme::from_palette(styler.palette());
        self.render_handle.set_styler(styler);
        self.request_map_redraw();
    }

    fn handle_resize(&mut self, cols: u16, rows: u16) {
        self.map.resize(cols, rows);
        self.render_handle
            .request_resize(self.map.width(), self.map.height());
        self.request_map_redraw();
    }

    fn request_map_redraw(&mut self) {
        if !self.map.is_running() {
            return;
        }
        let viewport = self.map.viewport();
        self.render_handle.request_draw(viewport);
    }
}

/// Composition root for the tile subsystem.
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
        crate::map::tile::TileCache::new(boxed, rx, config.cache.tiles, config.cache.memory_tiles),
        attribution,
    )
}

/// Composition root for plugins. **This is the only function that
/// names concrete plugin modules by type path**; `App` itself is
/// plugin-agnostic. Order matters: the palette is installed last so
/// its default provider can harvest every other plugin's palette
/// entries.
fn build_registrar(
    config: &Config,
    nominatim: Arc<NominatimClient>,
    attribution: Option<String>,
    keymap: &KeyMap,
) -> Registrar {
    use std::rc::Rc;

    let mut r = Registrar::default();

    crate::plugin::info::register(nominatim.clone(), &mut r);
    crate::plugin::scalebar::register(&mut r);
    crate::plugin::attribution::register(attribution, &mut r);
    crate::plugin::search::register(nominatim, &mut r);
    crate::plugin::wiki::register(&config.render.language, config.wiki.limit, &mut r);
    crate::plugin::here::register(
        config.geoip.endpoint.clone(),
        config.geoip.timeout_ms,
        &mut r,
    );
    crate::plugin::export::register(&mut r);
    crate::plugin::aircraft::register(&mut r);
    crate::plugin::iss::register(&mut r);
    crate::plugin::quake::register(&mut r);

    // Help needs to know the other plugins' activation hints, so build
    // its text after them (but before palette install, since palette
    // harvests help's palette entry too).
    let plugin_help_entries: Vec<(String, String)> = r
        .palette_entries
        .iter()
        .filter(|e| !e.hint.is_empty())
        .map(|e| (e.hint.clone(), e.label.clone()))
        .collect();
    let help_text = Rc::new(crate::plugin::help::HelpText::build(
        keymap,
        &plugin_help_entries,
    ));
    crate::plugin::help::register(help_text, &mut r);

    // Palette is a built-in, not a plugin. `install` drains every
    // palette_entry contributed above and bakes them into the default
    // provider — so it must run after every plugin::*::register call.
    crate::palette::install(keymap, &mut r);

    r
}
