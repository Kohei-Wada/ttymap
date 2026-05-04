//! App layer — the loop driver above [`Dispatcher`].
//!
//! `App` drains [`AppEvent`]s off the unified `mpsc` bus, forwards
//! them to [`Dispatcher`] (the GoF Receiver — see
//! [`dispatcher`](mod@dispatcher)), and asks ratatui to paint each
//! iteration. State that mutates in response to commands lives on
//! `Dispatcher`; only the rendered `MapFrame` and the input
//! [`MouseAdapter`] live here, plus the bus poll-timeout.
//!
//! `App` doesn't own subsystems. Threads (render / input / frame
//! timer), the bus, and the channel are constructed by `main` at the
//! composition root and handed in. The App just runs the
//! per-iteration handler the loop calls into.
//!
//! Focus/modal state lives on [`Compositor`] — owned by `Dispatcher`,
//! borrowed by `App::render_into` for paint.

pub mod event;
pub mod frame_timer;
pub mod ui;

use crate::core::Dispatcher;
pub use event::AppEvent;

use std::io;

use crossterm::event::{Event, KeyCode, KeyModifiers};
use log::{debug, info};

use crate::UserCommand;
use crate::config::Config;
use crate::core::compositor::{BaseLayer, Compositor};
use crate::core::map::MapHandle;
use crate::core::map::render::frame::MapFrame;
pub use crate::input::KeybindingOverrides;
use crate::input::{KeyMap, MouseAdapter};
use crate::lua::LuaSubsystem;
use crate::theme::ThemeId;

pub struct App {
    /// GoF Receiver for `UserCommand`. Owns the state that mutates
    /// in response to commands; `App` is the loop driver above it.
    /// See [`Dispatcher`].
    dispatcher: Dispatcher,
    /// Latest rendered map snapshot drained from the render thread.
    /// `None` until the first frame arrives. Lives on App (not
    /// Dispatcher) because it is the *rendered product* that App
    /// displays — Dispatcher reads it as `Option<&MapFrame>` only on
    /// the `ExportFrame` arm.
    map_frame: Option<MapFrame>,
    /// Mouse-event translator. App owns this because `handle_input`
    /// is the only consumer.
    mouse: MouseAdapter,
    /// Main event-loop wake interval. Derived from
    /// `ttymap.opt.runtime.poll_timeout_ms` at startup. `pub` getter
    /// because `main` reads it to align the input thread / frame
    /// timer cadences.
    poll_timeout: std::time::Duration,
}

impl App {
    /// Build the App.
    ///
    /// Composition root (`main`) builds every subsystem upstream and
    /// hands them in: the map subsystem as [`MapHandle`], the Lua
    /// plugin subsystem as [`LuaSubsystem`] (already with the palette
    /// installed). App just consumes them — its only own work is
    /// wiring the compositor base layer and forwarding the relevant
    /// pieces to [`Dispatcher::new`].
    pub fn new(
        config: Config,
        keymap: KeyMap,
        theme_id: ThemeId,
        map: MapHandle,
        lua: LuaSubsystem,
    ) -> Self {
        let LuaSubsystem {
            handle: lua,
            activations,
            plugin_hints,
            // `palette_entries` was already drained by
            // `palette::install` from main; nothing left for App to
            // consume.
            palette_entries: _,
        } = lua;

        // Compositor bootstraps with the BaseLayer (keymap +
        // activation dispatch) at index 0. Every subsequent modal is
        // pushed on top.
        let mut compositor = Compositor::new();
        compositor.push(Box::new(BaseLayer::new(keymap, activations, plugin_hints)));

        App {
            dispatcher: Dispatcher::new(
                theme_id,
                map,
                lua,
                compositor,
                config.runtime.sidebar_width,
                std::time::Duration::from_millis(config.runtime.overlay_redraw_ms),
            ),
            map_frame: None,
            mouse: MouseAdapter::default(),
            poll_timeout: std::time::Duration::from_millis(config.runtime.poll_timeout_ms),
        }
    }

    /// The configured idle wake-up interval — `main` reads this when
    /// spinning up the input thread / frame timer so they share the
    /// same cadence.
    pub fn poll_timeout(&self) -> std::time::Duration {
        self.poll_timeout
    }

    /// Drive the per-iteration event loop until `Dispatcher` flips
    /// `running` off.
    ///
    /// The app owns the iteration shape (housekeeping → drain
    /// queue → poll components → render → throttle overlay redraw)
    /// because the ordering between those steps is an app-level
    /// concern, not a wiring concern. `main` stays the composition
    /// root: it builds the bus, the channel, and the off-thread
    /// subsystems, then hands them in here as borrows.
    pub fn run(
        &mut self,
        terminal: &mut ratatui::DefaultTerminal,
        event_rx: &std::sync::mpsc::Receiver<AppEvent>,
        event_tx: &std::sync::mpsc::Sender<AppEvent>,
    ) -> io::Result<()> {
        self.dispatcher.dispatch_initial_redraw();

        while self.dispatcher.is_running() {
            // Park on the unified bus until any source produces an
            // event; drain any further buffered events non-blockingly
            // so a burst doesn't push the paint behind.
            match event_rx.recv() {
                Ok(event) => self.handle_event(event, event_tx),
                Err(_) => break,
            }
            while let Ok(event) = event_rx.try_recv() {
                self.handle_event(event, event_tx);
            }

            // Component poll: any `win.emit(msg)` inside fires onto
            // the bus directly. Same-iteration `try_recv` ran above
            // already; an emission here will be picked up next
            // iteration.
            self.dispatcher.poll_compositor(self.map_frame.as_ref());

            // Drain Lua-enqueued ops *before* render so that ops
            // emitted by handler / palette / keybind callbacks during
            // event handling apply this frame. on_tick-emitted ops
            // fire during `render_into` below — those land in the
            // buffer and drain at the start of the *next* iteration's
            // `poll_compositor`, with the same one-frame visibility
            // lag as the prior CloseFlag-via-poll design.
            self.dispatcher.apply_lua_ops(self.map_frame.as_ref());

            // Render a frame. Inside `ui::draw`, the per-frame Lua
            // `tick` event fires against the live MapApi.
            self.render_into(terminal)?;

            // If plugin `on_tick` callbacks pushed polylines, throttle
            // the redraw request to the configured interval.
            if self.dispatcher.overlay_should_redraw() {
                self.dispatcher.request_map_redraw();
            }
        }

        Ok(())
    }

    /// Apply one event drained off the unified queue. Each variant
    /// has a small fixed handler and the work is bounded — long Lua
    /// callbacks notwithstanding, the loop never sits inside this
    /// for more than the time a single dispatch needs.
    fn handle_event(&mut self, event: AppEvent, event_tx: &std::sync::mpsc::Sender<AppEvent>) {
        match event {
            AppEvent::Command(msg) => self.dispatcher.dispatch(msg, self.map_frame.as_ref()),
            AppEvent::FrameReady(frame) => {
                self.map_frame = Some(frame);
                self.dispatcher.notify_frame_ready();
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
                    let _ = event_tx.send(AppEvent::Command(UserCommand::Quit));
                } else {
                    debug!("key event: {:?}", key_event.code);
                    self.dispatcher
                        .handle_key_event(key_event, self.map_frame.as_ref());
                }
            }
            Event::Resize(cols, rows) => {
                info!("resize: {}x{}", cols, rows);
                let _ = event_tx.send(AppEvent::Command(UserCommand::Resize(cols, rows)));
            }
            Event::Mouse(mouse) => {
                for msg in self.mouse.translate(mouse) {
                    let _ = event_tx.send(AppEvent::Command(msg));
                }
            }
            _ => {}
        }
    }

    /// Single per-iteration draw. The `tick` bus event fires from
    /// inside `ui::draw` against the live `MapApi` (see `ui::draw`).
    fn render_into(&mut self, terminal: &mut ratatui::DefaultTerminal) -> io::Result<()> {
        let ctx = self.dispatcher.context();
        // Field-disjoint borrows so the closure can hold immutable
        // refs alongside the mutable `overlay_sink`.
        let map_frame = self.map_frame.as_ref();
        let compositor = &self.dispatcher.compositor;
        let lua = &self.dispatcher.lua;
        let ui_theme = &self.dispatcher.ui_theme;
        let sidebar_open = self.dispatcher.sidebar.open;
        let sidebar_width = self.dispatcher.sidebar.width;
        let overlay_sink = self.dispatcher.overlay.sink_mut();
        terminal.draw(|f| {
            crate::app::ui::draw(
                f,
                crate::app::ui::DrawInputs {
                    map_frame,
                    compositor,
                    lua,
                    theme: ui_theme,
                    ctx: &ctx,
                    overlay_sink,
                    sidebar_open,
                    sidebar_width,
                },
            )
        })?;
        Ok(())
    }
}
