//! Application event loop and central message dispatcher.
//!
//! [`App`] is the sole **Receiver** in the GoF Command pattern: every
//! invoker (keymap, palette, plugins, mouse adapter) returns
//! `Vec<AppMsg>` and only `App::dispatch` executes them.
//!
//! Focus/modal state lives on [`Compositor`] — a stack of
//! [`Component`]s that replaced the old `FocusManager` + `Plugin`
//! trilogy. Components render map overlays through
//! `Component::paint_on_map`. The compositor never names a concrete
//! plugin type — it carries only `Box<dyn Component>`s populated by
//! the Lua dispatcher at composition time.

mod mouse;
pub mod msg;

pub use msg::AppMsg;

use std::io;
use std::sync::Arc;

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use log::{debug, info};

use crate::compositor::{BaseLayer, Compositor, Context, Registrar};
use crate::config::Config;
use crate::keymap::KeyMap;
use crate::map::render::frame::MapFrame;
use crate::map::render::pipeline::RenderPipeline;
use crate::map::render::thread::RenderHandle;
use crate::map::styler::Styler;
use crate::map::{Action, MapState, MapStateOptions};
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

        let (tile_cache, wake_rx) = build_tile_cache(&config);
        let keymap = KeyMap::with_overrides(&config.keymap);
        let theme_id = ThemeId::from_name(&config.render.style);
        // `_lua_shared` is kept alive on the App so every Lua plugin's
        // host accessor (`host:plugin_palette_entries()` etc.) keeps
        // reading the live snapshot for the program lifetime.
        let BuiltRegistrar {
            registrar,
            plugin_hints,
        } = build_registrar(&config, tile_cache.attribution(), &keymap);
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
        let render_handle = RenderHandle::spawn(pipeline, wake_rx);

        // Compositor bootstraps with the BaseLayer (keymap +
        // activation dispatch) at index 0. Every subsequent modal is
        // pushed on top.
        let mut compositor = Compositor::new();
        compositor.push(Box::new(BaseLayer::new(
            keymap,
            registrar.activations,
            plugin_hints,
        )));
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

            // Drain per-tick messages from compositor components.
            // Borrows &mut self transitively through dispatch, so
            // collect into a Vec first.
            let ctx = self.context();
            let compositor_msgs = self.compositor.poll(&ctx);
            for msg in compositor_msgs {
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

            // Idle wake rate. crossterm's `event::poll` blocks
            // until an input event arrives or this elapses, so the
            // real cost is bounded latency for things that aren't
            // event-driven: a freshly produced render frame (we
            // drain those non-blockingly at the top of the loop)
            // and any plugin whose `poll()` touches render output
            // (e.g. a counter). 16 ms = 60 Hz, indistinguishable
            // from instant for human perception, an order of
            // magnitude less idle CPU than the previous 4 ms.
            let mut poll_timeout = Duration::from_millis(16);
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
///
/// Wires the three-layer pipeline plus the optional disk fast path:
///
/// ```text
///                   ┌── render-thread disk fast path ──────────────────┐
///                   ▼                                                   │
///   FetchLane<F>  ──bytes──▶  decoder thread  ──DecodedTile──▶  TileCache
/// ```
///
/// where `F` is `DiskCachedFetcher<HttpFetcher>` when disk cache is
/// enabled, else just `HttpFetcher`. The fast path lets `TileCache`
/// read disk synchronously and push bytes directly to the decoder,
/// skipping the worker queue.
///
/// Backend dispatch happens here: a future MBTiles / PMTiles backend
/// would pick a different `TileFetcher`. `FetchLane` provides queue
/// / dedup / priority for any of them; `decoder::spawn_decoder` and
/// the cache are backend-agnostic.
pub(crate) fn build_tile_cache(
    config: &Config,
) -> (crate::map::tile::TileCache, crossbeam_channel::Receiver<()>) {
    use directories::ProjectDirs;
    use std::fs;

    use crate::map::tile::cache::DiskFastPath;
    use crate::map::tile::decoder;
    use crate::map::tile::fetch::{DiskCachedFetcher, FetchLane, HttpFetcher, TileFetchLane};

    /// Worker count for the HTTP backend. HTTP is I/O-bound, so a
    /// small pool covers the typical visible-tile + prefetch fan-out
    /// without saturating the upstream.
    const HTTP_WORKERS: usize = 6;

    let cache_dir = if config.cache.tiles {
        ProjectDirs::from("", "", "ttymap").map(|proj_dirs| {
            let dir = proj_dirs.cache_dir().to_path_buf();
            let _ = fs::create_dir_all(&dir);
            dir
        })
    } else {
        None
    };

    let (bytes_tx, bytes_rx) = std::sync::mpsc::channel();
    let http = HttpFetcher::new();

    // The lane wraps an HTTP fetcher; if disk cache is enabled, layer
    // a `DiskCachedFetcher` decorator on top so worker-side hits
    // short-circuit the network and on miss we write through.
    let lane: Box<dyn TileFetchLane> = match cache_dir.clone() {
        Some(dir) => Box::new(FetchLane::new(
            DiskCachedFetcher::new(http, dir),
            HTTP_WORKERS,
            bytes_tx.clone(),
        )),
        None => Box::new(FetchLane::new(http, HTTP_WORKERS, bytes_tx.clone())),
    };

    let (decoded_rx, wake_rx, _decoder_handle) = decoder::spawn_decoder(bytes_rx);

    // The render-thread fast path: on a memory miss the cache reads
    // and decodes the file synchronously, putting the tile into the
    // LRU in the same render frame. This restores pre-refactor disk-
    // hit responsiveness; HTTP fetches still flow through the slow
    // lane below.
    let disk_fast_path = cache_dir.map(|cache_dir| DiskFastPath { cache_dir });

    (
        crate::map::tile::TileCache::new(
            lane,
            decoded_rx,
            config.cache.memory_tiles,
            disk_fast_path,
        ),
        wake_rx,
    )
}

/// Composition root for plugins. **This is the only function that
/// names concrete plugin modules by type path**; `App` itself is
/// plugin-agnostic. Order matters: the palette is installed last so
/// its default provider can harvest every other plugin's palette
/// entries.
/// Tuple-struct carrier so [`App::new`] can keep the plugin hints
/// alive across the call to [`build_registrar`]. The hints would
/// otherwise be unreachable, since [`crate::palette::install`]
/// `mem::take`s `Registrar.palette_entries` before returning.
struct BuiltRegistrar {
    registrar: Registrar,
    plugin_hints: Vec<(&'static str, &'static str)>,
}

fn build_registrar(
    config: &Config,
    attribution: Option<String>,
    keymap: &KeyMap,
) -> BuiltRegistrar {
    use std::sync::Arc;

    let mut r = Registrar::default();

    // Build the shared runtime-data carrier once. Every Lua plugin
    // (bundled and user) sees the same `host:*` accessor surface;
    // there is no per-plugin Rust glue, no per-plugin upvalue
    // injection. Adding a new bundled plugin is one entry in
    // `lua::BUILTIN_SCRIPTS`; adding a user plugin is one file in
    // `~/.config/ttymap/plugins/`.
    let shared = Arc::new(crate::lua::host::LuaHostShared::new(
        attribution,
        config.geoip.endpoint.clone(),
        keymap_entries(keymap),
    ));

    // Bundled plugins (every `.lua` under `src/lua/scripts/`) always
    // register — disabling one is a source edit. The dispatcher reads
    // each script's own activation/kind/key/label metadata, so chrome
    // overlays, palette toggles, key binds, and the search palette
    // provider all flow through one path.
    crate::lua::register_builtin_plugins(shared.clone(), &mut r);

    // User plugins from `~/.config/ttymap/plugins/*.lua`. Same
    // dispatcher, same host accessors. Each script controls its own
    // activation via `module.enabled` — drop a file in, flip a
    // boolean, no rebuild and no TOML.
    crate::lua::register_user_plugins(shared.clone(), &mut r);

    // Snapshot every plugin's palette entries into the shared carrier.
    // Help reads this lazily (at render time) via
    // `host:plugin_palette_entries()`, so it sees every sibling
    // regardless of registration order.
    let palette_entries: Vec<(String, String)> = r
        .palette_entries
        .iter()
        .filter(|e| !e.hint.is_empty())
        .map(|e| (e.hint.clone(), e.label.clone()))
        .collect();
    shared.set_palette_entries(palette_entries.clone());

    // Harvest the BaseLayer's footer hints. Has to happen *before*
    // `palette::install` because that call `mem::take`s
    // `r.palette_entries`. Leak each pair once so they satisfy
    // [`Component::footer_hints`]'s `&'static str` contract — bounded
    // by the plugin count. A plugin opts into a footer entry by
    // setting `module.footer_hint`; the host does no guessing from
    // `label`. No `footer_hint` ⇒ no footer slot.
    let plugin_hints: Vec<(&'static str, &'static str)> = r
        .palette_entries
        .iter()
        .filter(|e| !e.hint.is_empty())
        .filter_map(|e| {
            let footer = e.footer_hint.as_ref()?;
            let key: &'static str = Box::leak(e.hint.clone().into_boxed_str());
            let label: &'static str = Box::leak(footer.clone().into_boxed_str());
            Some((key, label))
        })
        .collect();

    // Palette is a built-in, not a plugin. `install` drains every
    // palette_entry contributed above and bakes them into the default
    // provider — so it must run after every plugin's register call.
    crate::palette::install(keymap, &mut r);

    // `shared` is kept alive via Arc clones inside every LuaComponent
    // / LuaPaletteProvider — dropping it here is fine.
    drop(shared);

    BuiltRegistrar {
        registrar: r,
        plugin_hints,
    }
}

/// Build the `(key-binding, action-label)` pairs that the help plugin
/// surfaces via `host:keymap_entries()`. Live data — runtime keymap
/// overrides surface here.
fn keymap_entries(keymap: &KeyMap) -> Vec<(String, String)> {
    use crate::map::Action;
    Action::all_listed()
        .iter()
        .filter_map(|action| {
            let keys = keymap.keys_for(&AppMsg::Map(action.clone()));
            if keys.is_empty() {
                None
            } else {
                Some((keys.join(", "), action.label().to_string()))
            }
        })
        .collect()
}
