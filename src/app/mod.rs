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

pub mod event;
mod input_thread;
mod mouse;
pub mod msg;

pub use event::AppEvent;
pub use msg::AppMsg;

use std::io;
use std::sync::Arc;

use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use log::{debug, info};

use crate::compositor::{BaseLayer, Compositor, Context, Registrar};
use crate::config::Config;
use crate::keymap::{KeyMap, KeybindingOverrides};
use crate::lua::LuaTickRegistry;
use crate::lua::ttymap::LuaHostHandles;
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
    /// Per-frame tick dispatcher. Populated at startup from
    /// `Registrar.tick_registry` (every `ttymap.api.frame.on_tick(fn)`
    /// call across all plugin scripts lands here) and ticked once per
    /// frame against the live `MapApi`, before the compositor's
    /// `paint_on_map`.
    tick_registry: LuaTickRegistry,
    /// Ephemeral polyline overlays pushed by Lua plugins during the
    /// current frame's `on_tick` pass. Drained into the next
    /// `RenderTask::Draw` immediately after `ui::draw` returns — so
    /// the Lua side fire-and-forgets every frame and the render thread
    /// always gets the freshest set.
    overlay_sink: Vec<crate::map::render::overlay::UserPolyline>,
    /// Setup-state [`LuaHostHandles`] for every Lua plugin script
    /// (palette providers, plugin components, plugin loops, and any
    /// `ttymap.api.window.open` / `palette.open` callers). Each
    /// entry's `push_rx` is drained once per frame so queued
    /// components reach `compositor.push`. `Box<dyn Component>` is
    /// `!Send`, hence one channel per plugin (kept on main thread).
    lua_host_handles: Vec<LuaHostHandles>,
    /// Unified [`AppEvent`] receiver. Every fire-and-forget Lua intent
    /// (wrapped as [`AppEvent::Intent`]), every completed render frame
    /// from the render thread (wrapped as [`AppEvent::FrameReady`]),
    /// and every terminal event from the input thread (wrapped as
    /// [`AppEvent::Input`]) arrives here. Drained once per main-loop
    /// iteration; the loop parks on this channel so idle CPU is
    /// dominated by `poll_timeout` rather than per-tick polling.
    event_rx: std::sync::mpsc::Receiver<AppEvent>,
    /// Sender side of the unified queue. Cloned into the input thread
    /// at `run()` time. Render thread + Lua plugins received their
    /// own clones at construction; this one stays on the App so the
    /// input thread can be (re)spawned without re-plumbing.
    event_tx: std::sync::mpsc::Sender<AppEvent>,
    /// Latest mouse cursor position in absolute terminal cells.
    /// `None` until the first mouse event arrives (or always, on
    /// terminals without mouse support). Surfaced to components via
    /// [`Context`](crate::compositor::Context) so plugins can build
    /// cursor-aware overlays.
    cursor: Option<(u16, u16)>,
    /// Timestamp of the last overlay-driven `request_map_redraw`. Used
    /// to rate-limit redraws when a plugin pushes polyline overlays
    /// every tick — without this, each push triggers a full tile re-
    /// render at the main loop's ~60Hz cadence (`event::poll(16ms)`),
    /// which is wasted work since tile data does not change between
    /// frames. Throttling to ~30Hz halves render-thread CPU while
    /// keeping animation visually smooth. User-event-driven redraws
    /// (pan, zoom, resize, theme change) bypass this check and fire
    /// immediately.
    last_overlay_redraw: std::time::Instant,
    /// Main event-loop wake interval. Derived from
    /// `ttymap.opt.runtime.poll_timeout_ms` at startup.
    poll_timeout: std::time::Duration,
    /// Minimum interval between overlay-driven redraws. Derived from
    /// `ttymap.opt.runtime.overlay_redraw_ms` at startup.
    overlay_redraw_interval: std::time::Duration,
}

impl App {
    pub fn new(config: Config, keymap_overrides: KeybindingOverrides) -> Self {
        let (cols, rows) = crossterm::terminal::size().unwrap_or((80, 24));
        let (width, height) = crate::map::render::canvas_size(cols, rows);

        info!(
            "terminal size: {}x{}, canvas: {}x{}",
            cols, rows, width, height
        );

        let (tile_cache, wake_rx) = build_tile_cache(&config);
        let keymap = KeyMap::with_overrides(&keymap_overrides);
        let theme_id = ThemeId::from_name(&config.render.style);

        // Unified [`AppEvent`] channel shared by every Lua plugin and
        // the render thread. Each plugin's `HostMap` / export closure
        // clones `event_tx` and pushes fire-and-forget intents wrapped
        // as `AppEvent::Intent(...)`; the render thread pushes
        // `AppEvent::FrameReady(frame)` once a render completes. The
        // App holds the matching Receiver and drains everything in
        // arrival order through a single loop per frame.
        let (event_tx, event_rx) = std::sync::mpsc::channel::<AppEvent>();

        // `_lua_shared` is kept alive on the App so every Lua plugin's
        // host accessor (`ttymap.help:palette_entries()` etc.) keeps
        // reading the live snapshot for the program lifetime.
        let BuiltRegistrar {
            mut registrar,
            plugin_hints,
        } = build_registrar(&config, tile_cache.attribution(), &keymap, event_tx.clone());
        // Lift the per-frame tick dispatcher off the registrar before
        // the rest is consumed (activations / palette_entries move
        // into the compositor below). Owned on App so the per-frame
        // `tick` call has direct access without threading the
        // registrar reference through.
        let tick_registry = std::mem::take(&mut registrar.tick_registry);
        // Same pattern for the setup-state handle pool: the App
        // drains these per frame so plugins' `ttymap.map:jump` /
        // `api.frame.export` / `api.window.open` / `api.palette.open`
        // calls (all running on the setup state) reach the compositor
        // and the app dispatch table.
        let lua_host_handles = std::mem::take(&mut registrar.lua_host_handles);
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
        let render_handle = RenderHandle::spawn(pipeline, wake_rx, event_tx.clone());

        // Compositor bootstraps with the BaseLayer (keymap +
        // activation dispatch) at index 0. Every subsequent modal is
        // pushed on top.
        let mut compositor = Compositor::new();
        compositor.push(Box::new(BaseLayer::new(
            keymap,
            registrar.activations,
            plugin_hints,
        )));

        App {
            map,
            render_handle,
            map_frame: None,
            mouse: MouseAdapter::default(),
            compositor,
            theme_id,
            ui_theme,
            tick_registry,
            lua_host_handles,
            event_rx,
            event_tx,
            cursor: None,
            overlay_sink: Vec::new(),
            last_overlay_redraw: std::time::Instant::now(),
            poll_timeout: std::time::Duration::from_millis(config.runtime.poll_timeout_ms),
            overlay_redraw_interval: std::time::Duration::from_millis(
                config.runtime.overlay_redraw_ms,
            ),
        }
    }

    pub fn run(&mut self) -> io::Result<()> {
        let mut terminal = ratatui::init();
        crossterm::execute!(io::stdout(), crossterm::event::EnableMouseCapture)?;

        // Spawn the input thread *after* terminal setup so it never
        // reads from a non-raw stdin. The handle's `Drop` joins on
        // shutdown — it's tied to this scope so the thread is cleaned
        // up before `ratatui::restore` runs at the end of the function.
        let _input = input_thread::InputHandle::spawn(self.event_tx.clone(), self.poll_timeout);

        info!("event loop started");
        self.dispatch(AppMsg::Map(Action::Redraw));

        while self.map.is_running() {
            // Refresh per-plugin getter mirrors and drain `push_rx`
            // (which carries `Box<dyn Component>`, kept off the unified
            // queue because `Component` is `!Send`). Components queued
            // via `ttymap.api.window.open` / `palette.open` land on
            // the compositor stack here so the same tick's
            // `compositor.poll` already sees them.
            self.refresh_lua_host_state();

            // Park on the unified queue. `recv_timeout` returns either
            // an event or a timeout — in either case we fall through
            // to the per-iteration draw, so animation plugins still
            // tick at ~`poll_timeout` cadence even with no input. The
            // first event is processed inline; subsequent buffered
            // events drain non-blockingly so we never paint behind a
            // burst.
            match self.event_rx.recv_timeout(self.poll_timeout) {
                Ok(event) => self.handle_event(event),
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            }
            while let Ok(event) = self.event_rx.try_recv() {
                self.handle_event(event);
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
                    &self.tick_registry,
                    &self.ui_theme,
                    &ctx,
                    &mut self.overlay_sink,
                )
            })?;

            // Plugins push polylines into `overlay_sink` from their `on_tick`
            // callbacks during `ui::draw`. Drain unconditionally so the next
            // render task carries them — without this the sink grows every
            // frame any plugin calls `map:polyline`. The render thread's
            // `drain_tasks` collapses redundant Draw tasks to the latest, so
            // this doesn't cause N renders per second; it just guarantees the
            // freshly-pushed polylines reach the next render.
            //
            // Rate-limited to ~30Hz: plugins can push every tick at 60Hz, but
            // we only trigger a full tile re-render at most every 33ms. Polylines
            // accumulate in `overlay_sink` between redraws and are all delivered
            // in the next triggered Draw. User-event-driven redraws (pan, zoom,
            // resize, theme change) bypass this check and always fire immediately.
            if !self.overlay_sink.is_empty() {
                let now = std::time::Instant::now();
                if now.duration_since(self.last_overlay_redraw) >= self.overlay_redraw_interval {
                    self.request_map_redraw();
                    self.last_overlay_redraw = now;
                }
            }
        }

        info!("event loop ended");
        // Stop the input thread *before* restoring the terminal so it
        // never reads from a non-raw stdin during teardown. Default
        // Drop ordering (locals dropped after the explicit cleanup
        // below) would otherwise leave the thread polling against a
        // terminal in cooked mode for the join window.
        drop(_input);
        crossterm::execute!(io::stdout(), crossterm::event::DisableMouseCapture)?;
        ratatui::restore();
        info!("terminal restored, exiting");

        Ok(())
    }

    /// Drain every Lua plugin's setup-state receivers once.
    ///
    /// Before draining, refreshes the host-shared `center` / `zoom`
    /// cells each plugin's `ttymap.map:center()` / `:zoom()` reads
    /// from. Doing this once at the App level — instead of from
    /// inside each `LuaWindowComponent` dispatch — means the values
    /// are correct for *every* callback path (palette invoke,
    /// register_keybind, on_tick), not just paths that go through
    /// an active window.
    ///
    /// Per-plugin housekeeping: refresh `center` / `zoom` mirrors that
    /// `ttymap.map:center()` / `:zoom()` read, and drain each plugin's
    /// `push_rx` so any component queued via `ttymap.api.window.open`
    /// / `palette.open` reaches the compositor stack before the
    /// current tick's `compositor.poll`. Stays per-plugin because
    /// `Box<dyn Component>` is `!Send` and the channel must remain on
    /// the main thread.
    ///
    /// Intent flow (`map:jump`, `frame.export`, …) is **not** drained
    /// here — those reach the App through the unified `event_rx`.
    fn refresh_lua_host_state(&mut self) {
        let center = self.map.center();
        let zoom = self.map.zoom();
        for handles in &self.lua_host_handles {
            if let Ok(mut cell) = handles.center.lock() {
                *cell = center;
            }
            if let Ok(mut cell) = handles.zoom.lock() {
                *cell = zoom;
            }
            while let Ok(component) = handles.push_rx.try_recv() {
                self.compositor.push(component);
            }
        }
    }

    /// Apply one event drained off the unified queue. Each variant
    /// has a small fixed handler and the work is bounded — long Lua
    /// callbacks notwithstanding, the loop never sits inside this
    /// for more than the time a single dispatch needs.
    fn handle_event(&mut self, event: AppEvent) {
        match event {
            AppEvent::Intent(msg) => self.dispatch(msg),
            AppEvent::FrameReady(frame) => {
                self.map_frame = Some(frame);
            }
            AppEvent::Input(input) => self.handle_input(input),
        }
    }

    /// Classify a raw terminal event and dispatch downstream messages.
    /// Same logic as the prior inline `crossterm::event::poll` block —
    /// just relocated so it can run from the unified-queue drain.
    fn handle_input(&mut self, event: Event) {
        match event {
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

    fn handle_key(&mut self, key: KeyEvent) {
        let ctx = self.context();
        let msgs = self.compositor.handle_event(key, &ctx);
        for msg in msgs {
            self.dispatch(msg);
        }
    }

    fn context(&self) -> Context {
        Context {
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
        let overlays = std::mem::take(&mut self.overlay_sink);
        self.render_handle.request_draw(viewport, overlays);
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
    event_tx: std::sync::mpsc::Sender<AppEvent>,
) -> BuiltRegistrar {
    use std::sync::Arc;

    let mut r = Registrar::default();

    // Build the shared runtime-data carrier once. Every Lua plugin
    // (bundled and user) sees the same `ttymap.*` accessor surface;
    // there is no per-plugin Rust glue, no per-plugin upvalue
    // injection. Adding a new bundled plugin is one file under
    // `runtime/plugin/`; adding a user plugin is one file in
    // `~/.config/ttymap/plugin/`.
    let shared = Arc::new(crate::lua::LuaHostShared::new(
        attribution,
        config.geoip.endpoint.clone(),
        keymap_entries(keymap),
    ));

    // Bundled plugins (every `*.lua` under each runtime layer's
    // `plugin/`) always register — disabling one is an edit to the
    // script itself (`enabled = false` in the spec). Higher-priority
    // layers shadow lower ones by stem, so a user's
    // `~/.config/ttymap/plugin/wiki.lua` replaces the bundled `wiki`.
    // The dispatcher reads each script's own activation/kind/key/label
    // metadata, so chrome overlays, palette toggles, key binds, and
    // the search palette provider all flow through one path.
    //
    // `runtime_path()` was set once at startup by `main.rs` (or the
    // test harness via `ensure_runtime_path_for_tests`).
    //
    // User plugins live in `~/.config/ttymap/plugin/` — that's just
    // the xdg_config layer of `runtime_path()`, so the same walker
    // picks them up. Higher-priority layers shadow lower ones by
    // stem (env > manifest > xdg_config > xdg_data).
    let runtime_path = crate::lua::runtime_path();
    crate::lua::register_builtin_plugins(
        runtime_path,
        &config.plugins.disable,
        shared.clone(),
        event_tx,
        &mut r,
    );

    // Plugin metadata for help is published to `shared.palette_entries`
    // directly during registration (see `lua::push_plugin_entry`), so
    // there's no harvest step here. Help reads the snapshot lazily at
    // render time via `ttymap.help:palette_entries()`.

    // Harvest the BaseLayer's footer hints. Has to happen *before*
    // `palette::install` because that call `mem::take`s
    // `r.palette_entries`. The footer slot is `[<key> <name>]` —
    // built directly from each entry's keybinding and `module.name`.
    // No keybinding ⇒ no footer slot. Leak the key string once to
    // satisfy [`Component::footer_hints`]'s `&'static str` contract;
    // `name` is already `&'static`.
    let plugin_hints: Vec<(&'static str, &'static str)> = r
        .palette_entries
        .iter()
        .filter(|e| !e.hint.is_empty())
        .map(|e| {
            let key: &'static str = Box::leak(e.hint.clone().into_boxed_str());
            (key, e.name)
        })
        .collect();

    // Palette is a built-in, not a plugin. `install` drains every
    // palette_entry contributed above and bakes them into the default
    // provider — so it must run after every plugin's register call.
    crate::palette::install(keymap, &mut r);

    // `shared` is kept alive via Arc clones inside every Lua plugin's
    // setup state and any `LuaPaletteProvider` it creates — dropping
    // the local handle here is fine.
    drop(shared);

    BuiltRegistrar {
        registrar: r,
        plugin_hints,
    }
}

/// Build the `(key-binding, action-label)` pairs that the help plugin
/// surfaces via `ttymap.help:keymap_entries()`. Live data — runtime keymap
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
