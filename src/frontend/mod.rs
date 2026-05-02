//! Frontend layer — receives [`AppEvent`]s, mutates state, and
//! draws.
//!
//! [`Frontend`] is the sole **Receiver** in the GoF Command pattern:
//! every invoker (keymap, palette, plugins, mouse adapter, render
//! thread, input thread, frame timer) emits onto the unified
//! [`AppEvent`] bus, and only `Frontend::dispatch` executes the
//! resulting `AppMsg`s. Component hooks express intent through
//! `Window::emit(msg)` which routes onto the same bus — no
//! synchronous "return `Vec<AppMsg>`" path remains.
//!
//! `Frontend` doesn't own subsystems. Threads (render / input /
//! frame timer), the bus, and the channel are constructed by `main`
//! at the composition root and handed in. The Frontend just runs
//! the per-iteration handler the loop calls into.
//!
//! Focus/modal state lives on [`Compositor`] — a stack of
//! [`Component`]s that replaced the old `FocusManager` + `Plugin`
//! trilogy. Components render map overlays through
//! `Component::paint_on_map`. The compositor never names a concrete
//! plugin type — it carries only `Box<dyn Component>`s populated by
//! the Lua dispatcher at composition time.

pub mod event;
pub mod frame_timer;
pub mod input_thread;
mod mouse;
pub mod msg;
pub mod ui;

pub use event::AppEvent;
pub use msg::AppMsg;

use std::io;
use std::sync::Arc;

use crossterm::event::{Event, KeyCode, KeyModifiers};
use log::{debug, info};

use crate::compositor::{BaseLayer, Compositor, Context, Registrar};
use crate::config::Config;
use crate::keymap::KeyMap;
pub use crate::keymap::KeybindingOverrides;
use crate::lua::LuaEventBus;
use crate::lua::ttymap::LuaHostHandles;
use crate::map::render::frame::MapFrame;
use crate::map::render::pipeline::RenderPipeline;
use crate::map::render::thread::{RenderClient, RenderHandle};
use crate::map::styler::Styler;
use crate::map::{Action, MapState, MapStateOptions};
use crate::theme::ThemeId;
use crate::theme::UiTheme;
use mouse::MouseAdapter;

pub struct Frontend {
    map: MapState,
    /// Cheap-clone command channel for the render thread. The thread
    /// itself (and its `Drop`-driven join) is owned by `main` as a
    /// peer subsystem to the App, mirroring how the input thread and
    /// frame timer live outside the App.
    render_client: RenderClient,
    /// Latest rendered map snapshot drained from the render thread.
    /// `None` until the first frame arrives. Owned here directly —
    /// no UiState wrapper now that built-in chrome lives in plugins.
    map_frame: Option<MapFrame>,
    mouse: MouseAdapter,
    compositor: Compositor,
    theme_id: ThemeId,
    ui_theme: UiTheme,
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

impl Frontend {
    /// Build the app state and the Lua event bus.
    ///
    /// The unified [`AppEvent`] channel is constructed by the caller
    /// (typically `main`) so the bus stays a wiring concern at the
    /// composition root rather than something `App` "owns". Each
    /// subsystem (render thread / Lua plugins / future input thread /
    /// frame timer) gets its own clone of `event_tx`; the App takes
    /// one too only because [`Compositor::poll`] / `Compositor::handle_event`
    /// pass it to `Window::emit` — same role as any other source.
    ///
    /// Returns `(App, LuaEventBus)` because the event bus is built
    /// during plugin registration but drained from outside the App
    /// (`ui::draw` for `dispatch_tick`, the run loop for the post-
    /// effect notification path).
    pub fn new(
        config: Config,
        keymap_overrides: KeybindingOverrides,
        event_tx: std::sync::mpsc::Sender<AppEvent>,
    ) -> (Self, RenderHandle, LuaEventBus) {
        let (cols, rows) = crossterm::terminal::size().unwrap_or((80, 24));
        let (width, height) = crate::map::render::canvas_size(cols, rows);

        info!(
            "terminal size: {}x{}, canvas: {}x{}",
            cols, rows, width, height
        );

        let (tile_cache, wake_rx) = crate::map::tile::build(&config);
        let keymap = KeyMap::with_overrides(&keymap_overrides);
        let theme_id = ThemeId::from_name(&config.render.style);

        // `_lua_shared` is kept alive on the App so every Lua plugin's
        // host accessor (`ttymap.help:palette_entries()` etc.) keeps
        // reading the live snapshot for the program lifetime.
        let BuiltRegistrar {
            mut registrar,
            plugin_hints,
        } = build_registrar(&config, tile_cache.attribution(), &keymap, event_tx.clone());
        // Lift the Lua event bus off the registrar before the rest
        // is consumed (activations / palette_entries move into the
        // compositor below). Returned by value so the caller (main)
        // owns it — the App is one of multiple parties that fires
        // events on it, no longer the bus's owner.
        let event_bus = std::mem::take(&mut registrar.event_bus);
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
        let render_client = render_handle.client();

        // Compositor bootstraps with the BaseLayer (keymap +
        // activation dispatch) at index 0. Every subsequent modal is
        // pushed on top.
        let mut compositor = Compositor::new();
        compositor.push(Box::new(BaseLayer::new(
            keymap,
            registrar.activations,
            plugin_hints,
        )));

        let app = Frontend {
            map,
            render_client,
            map_frame: None,
            mouse: MouseAdapter::default(),
            compositor,
            theme_id,
            ui_theme,
            lua_host_handles,
            cursor: None,
            overlay_sink: Vec::new(),
            last_overlay_redraw: std::time::Instant::now(),
            poll_timeout: std::time::Duration::from_millis(config.runtime.poll_timeout_ms),
            overlay_redraw_interval: std::time::Duration::from_millis(
                config.runtime.overlay_redraw_ms,
            ),
        };
        (app, render_handle, event_bus)
    }

    /// The configured idle wake-up interval — `main` reads this when
    /// spinning up the input thread / frame timer so they share the
    /// same cadence.
    pub fn poll_timeout(&self) -> std::time::Duration {
        self.poll_timeout
    }

    /// Whether the map state machine still wants the loop to keep
    /// running. The run loop checks this at the top of each iteration.
    pub fn is_running(&self) -> bool {
        self.map.is_running()
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
    ///
    /// Two-stage shape per variant: **execute** (state mutation —
    /// `dispatch`, `map_frame = ...`, etc.) followed by **notify**
    /// (broadcast a post-effect notification on the Lua event bus).
    /// Subscribers (Lua plugins via `ttymap.on_event`) only see the
    /// notification path; they read the resulting state through the
    /// usual `ttymap.map:*` accessors. Intent flow stays single-
    /// executor; the bus is observation only.
    pub fn handle_event(
        &mut self,
        event: AppEvent,
        event_bus: &LuaEventBus,
        event_tx: &std::sync::mpsc::Sender<AppEvent>,
    ) {
        match event {
            AppEvent::Intent(msg) => {
                let snapshot = msg.clone();
                self.dispatch(msg);
                self.notify_post_intent(&snapshot, event_bus);
            }
            AppEvent::FrameReady(frame) => {
                self.map_frame = Some(frame);
                event_bus.dispatch(crate::lua::registry::names::FRAME_READY, ());
            }
            AppEvent::Input(input) => self.handle_input(input, event_tx),
            // `Wake` exists purely to unblock `event_rx.recv()`. The
            // per-iteration draw + overlay-redraw rate-check below
            // already does whatever per-frame work is needed; no
            // extra handler logic belongs here. Distinct from the
            // Lua-side `"tick"` event which fires from inside draw.
            AppEvent::Wake => {}
        }
    }

    /// Broadcast post-effect notifications for the variants that
    /// plugins want to observe. Skips noisy / internal intents
    /// (`PanCells`, `CursorMoved`, `CycleFocus`, etc.) so the bus
    /// surface stays meaningful — bus events are "something
    /// observable happened to the app", not "every state mutation".
    fn notify_post_intent(&self, msg: &AppMsg, event_bus: &LuaEventBus) {
        use crate::lua::registry::names;
        match msg {
            AppMsg::Map(Action::Jump(ll)) => {
                event_bus.dispatch(names::MAP_JUMPED, (ll.lon, ll.lat));
            }
            AppMsg::Map(Action::SetZoom(z)) => {
                event_bus.dispatch(names::MAP_ZOOM_SET, *z);
            }
            AppMsg::Map(Action::FlyTo { center, zoom }) => {
                event_bus.dispatch(names::MAP_FLEW_TO, (center.lon, center.lat, *zoom));
            }
            AppMsg::SetTheme(new_id) => {
                event_bus.dispatch(names::THEME_CHANGED, new_id.name());
            }
            AppMsg::Resize(cols, rows) => {
                event_bus.dispatch(names::RESIZED, (*cols, *rows));
            }
            AppMsg::ExportFrame => {
                event_bus.dispatch(names::FRAME_EXPORTED, ());
            }
            // Noisy or internal — `PanCells`, `ZoomAt`, `CursorMoved`,
            // `CycleFocus`, the discrete `Pan*` keymap actions, and
            // `Quit` (the App is tearing down anyway) are deliberately
            // not broadcast. Adding them later is one match arm.
            _ => {}
        }
    }

    /// Classify a raw terminal event and dispatch downstream messages.
    /// Same logic as the prior inline `crossterm::event::poll` block —
    /// just relocated so it can run from the unified-queue drain.
    fn handle_input(&mut self, event: Event, event_tx: &std::sync::mpsc::Sender<AppEvent>) {
        match event {
            Event::Key(key_event) => {
                if key_event.modifiers.contains(KeyModifiers::CONTROL)
                    && key_event.code == KeyCode::Char('c')
                {
                    info!("Ctrl-C received, quitting");
                    let _ = event_tx.send(AppEvent::Intent(AppMsg::Map(Action::Quit)));
                } else {
                    debug!("key event: {:?}", key_event.code);
                    let ctx = self.context();
                    self.compositor.handle_event(key_event, &ctx, event_tx);
                }
            }
            Event::Resize(cols, rows) => {
                info!("resize: {}x{}", cols, rows);
                let _ = event_tx.send(AppEvent::Intent(AppMsg::Resize(cols, rows)));
            }
            Event::Mouse(mouse) => {
                for msg in self.mouse.translate(mouse) {
                    let _ = event_tx.send(AppEvent::Intent(msg));
                }
            }
            _ => {}
        }
    }

    fn context(&self) -> Context {
        Context {
            theme_id: self.theme_id,
            cursor: self.cursor,
        }
    }

    /// Per-iteration housekeeping the run loop runs **before**
    /// draining the event queue. Refreshes per-plugin `center` /
    /// `zoom` Mutexes and pushes any components Lua queued via
    /// `ttymap.api.window.open` / `palette.open` onto the
    /// compositor stack so the same iteration's `compositor.poll`
    /// already sees them.
    pub fn refresh_lua_host_state_per_tick(&mut self) {
        self.refresh_lua_host_state();
    }

    /// Drive a single `compositor.poll` pass. Components emitting
    /// intents through `Window::emit` route directly onto the bus —
    /// the run loop's same-iteration `try_recv` picks them up.
    pub fn poll_compositor(&mut self, event_tx: &std::sync::mpsc::Sender<AppEvent>) {
        let ctx = self.context();
        self.compositor.poll(&ctx, event_tx);
    }

    /// Single per-iteration draw. Owns the borrow gymnastics — main
    /// calls this with the terminal handle and the bus, App passes
    /// its own state into `ui::draw` without exposing internal
    /// fields. The `tick` bus event fires from inside `ui::draw`
    /// against the live `MapApi` (see `ui::draw`).
    pub fn render_into(
        &mut self,
        terminal: &mut ratatui::DefaultTerminal,
        event_bus: &LuaEventBus,
    ) -> io::Result<()> {
        let ctx = self.context();
        terminal.draw(|f| {
            crate::frontend::ui::draw(
                f,
                self.map_frame.as_ref(),
                &self.compositor,
                event_bus,
                &self.ui_theme,
                &ctx,
                &mut self.overlay_sink,
            )
        })?;
        Ok(())
    }

    /// After draw: if Lua plugins pushed polylines into
    /// `overlay_sink` during `on_tick` and the throttle interval
    /// has elapsed, queue a fresh `RenderTask::Draw` so the next
    /// frame carries them.
    pub fn tick_overlay_redraw(&mut self) {
        if !self.overlay_sink.is_empty() {
            let now = std::time::Instant::now();
            if now.duration_since(self.last_overlay_redraw) >= self.overlay_redraw_interval {
                self.request_map_redraw();
                self.last_overlay_redraw = now;
            }
        }
    }

    /// Initial dispatch fired by the run loop right after entering
    /// the loop — kicks off the very first render task so the
    /// terminal isn't blank waiting for input.
    pub fn dispatch_initial_redraw(&mut self) {
        self.dispatch(AppMsg::Map(Action::Redraw));
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
        self.render_client.set_styler(styler);
        self.request_map_redraw();
    }

    fn handle_resize(&mut self, cols: u16, rows: u16) {
        self.map.resize(cols, rows);
        self.render_client
            .request_resize(self.map.width(), self.map.height());
        self.request_map_redraw();
    }

    fn request_map_redraw(&mut self) {
        if !self.map.is_running() {
            return;
        }
        let viewport = self.map.viewport();
        let overlays = std::mem::take(&mut self.overlay_sink);
        self.render_client.request_draw(viewport, overlays);
    }
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
