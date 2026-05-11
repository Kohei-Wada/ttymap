//! App layer — the loop driver above [`Dispatcher`].
//!
//! `App` drains [`AppEvent`]s off the unified `mpsc` queue, forwards
//! each to `Dispatcher` (which owns every piece of mutable state),
//! then drains the events Dispatcher accumulated and publishes them
//! on the bus. That's the entire shape — there is no state on App
//! beyond the bus handle and the loop's wake interval.
//!
//! The two invariants:
//! - **Single publish site.** Only `App::publish_pending` calls
//!   `bus.publish`. Handlers never touch the bus; Dispatcher accumulates
//!   events into a buffer App drains.
//! - **State lives on Dispatcher.** App is a router. `MouseAdapter`,
//!   `MapFrame`, `LuaHandle` etc. all sit on Dispatcher; App reaches
//!   them only through method calls.
//!
//! `main` is the composition root: it builds the bus, the channel,
//! and the off-thread subsystems, then hands them in.

pub mod dispatcher;
pub mod event;
pub mod frame_timer;
mod frame_widget;
mod overlay;
mod sidebar;
pub mod ui;

use dispatcher::Dispatcher;
pub use event::AppEvent;

use std::io;
use std::rc::Rc;

use crate::compositor::{BaseLayer, Compositor};
use crate::config::Config;
use crate::event::EventBus;
use crate::input::KeyMap;
pub use crate::input::KeybindingOverrides;
use crate::lua::LuaSubsystem;
use crate::theme::ThemeId;
use ttymap_engine::map::MapHandle;

pub struct App {
    /// Sole owner of mutable App state and the event accumulator
    /// `publish_pending` drains. See [`Dispatcher`].
    dispatcher: Dispatcher,
    /// The Lua-agnostic pub/sub primitive. Only [`Self::publish_pending`]
    /// calls `publish` on it — the single fan-out site for the
    /// program.
    bus: Rc<EventBus>,
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
        builtin_activations: Vec<crate::compositor::Activation>,
        lua: LuaSubsystem,
    ) -> Self {
        let LuaSubsystem {
            handle: lua,
            bus,
            registry,
            footer_hints,
        } = lua;

        // Compositor bootstraps with the BaseLayer (keymap +
        // activation dispatch) at index 0. Every subsequent modal is
        // pushed on top. BaseLayer borrows the live `LuaRegistry`
        // so plugin `KeybindHandle:remove()` updates are visible on
        // the next keypress; built-in activations (today: just `:`
        // for the palette) are kept in their own Vec so plugins
        // can't accidentally shadow host shortcuts.
        let mut compositor = Compositor::new();
        compositor.push(Box::new(BaseLayer::new(
            keymap,
            builtin_activations,
            registry,
            footer_hints,
        )));

        App {
            dispatcher: Dispatcher::new(
                theme_id,
                map,
                lua,
                compositor,
                config.runtime.sidebar_width,
                std::time::Duration::from_millis(config.runtime.overlay_redraw_ms),
            ),
            bus,
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
    /// Shape: drain queue → poll components → apply Lua ops →
    /// publish pending → render → throttle overlay redraw. `main`
    /// stays the composition root: it builds the bus, the channel,
    /// and the off-thread subsystems, then hands them in here as
    /// borrows.
    pub fn run(
        &mut self,
        terminal: &mut ratatui::DefaultTerminal,
        event_rx: &std::sync::mpsc::Receiver<AppEvent>,
        event_tx: &std::sync::mpsc::Sender<AppEvent>,
    ) -> io::Result<()> {
        self.dispatcher.dispatch_initial_redraw();
        self.publish_pending();

        while self.dispatcher.is_running() {
            // Park on the unified queue until any source produces an
            // event; drain any further buffered events non-blockingly
            // so a burst doesn't push the paint behind.
            match event_rx.recv() {
                Ok(event) => self.handle_event(event, event_tx),
                Err(_) => break,
            }
            while let Ok(event) = event_rx.try_recv() {
                self.handle_event(event, event_tx);
            }

            // Component poll: any handler-returned `Op`s apply through
            // Dispatcher's accumulator; any `Op::Publish` lands in
            // `pending_events` and ships out at the next drain.
            self.dispatcher.poll_compositor();

            // Drain Lua-enqueued ops *before* render so that ops
            // emitted by handler / palette / keybind callbacks during
            // event handling apply this frame. on_tick-emitted ops
            // fire during `render_into` below — those land in the
            // buffer and drain at the start of the *next* iteration's
            // `poll_compositor`, with the same one-frame visibility
            // lag as the prior CloseFlag-via-poll design.
            self.dispatcher.apply_lua_ops();

            // Single bus-publish site for the entire program. Every
            // `pending_events` push from Dispatcher (dispatch arms,
            // accept_frame, forward_external_event, Op::Publish in
            // apply_ops) ships out here.
            self.publish_pending();

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

    /// Route one event into the right Dispatcher method. App does no
    /// state mutation itself — every arm is a one-line forward.
    fn handle_event(&mut self, event: AppEvent, event_tx: &std::sync::mpsc::Sender<AppEvent>) {
        match event {
            AppEvent::Command(msg) => self.dispatcher.dispatch(msg),
            AppEvent::FrameReady(frame) => self.dispatcher.accept_frame(frame),
            AppEvent::Input(input) => self.dispatcher.handle_input(input, event_tx),
            // `Wake` exists purely to unblock `event_rx.recv()`. The
            // per-iteration draw + overlay-redraw rate-check below
            // already does whatever per-frame work is needed; no
            // extra handler logic belongs here. Distinct from the
            // Lua-side `"tick"` event which fires from inside draw.
            AppEvent::Wake => {}
            // Cross-thread producers route through here so their
            // publish lands in the same accumulator App drains for
            // dispatch-produced events — preserving the
            // single-publish-site invariant.
            AppEvent::Bus(bus_event) => self.dispatcher.forward_external_event(bus_event),
        }
    }

    /// Drain every [`crate::event::Event`] Dispatcher accumulated
    /// and publish each onto the bus. The single fan-out point.
    fn publish_pending(&mut self) {
        for ev in self.dispatcher.drain_events() {
            self.bus.publish(ev);
        }
    }

    /// Single per-iteration draw. The `tick` bus event fires from
    /// inside `ui::draw` against the live `MapApi` (see `ui::draw`).
    fn render_into(&mut self, terminal: &mut ratatui::DefaultTerminal) -> io::Result<()> {
        let ctx = self.dispatcher.context();
        let inputs = self.dispatcher.draw_inputs(&ctx);
        terminal.draw(|f| crate::app::ui::draw(f, inputs))?;
        Ok(())
    }
}
