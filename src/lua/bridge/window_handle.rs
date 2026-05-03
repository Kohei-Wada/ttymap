//! `ttymap.api.window.open(spec) -> WindowHandle` — push a focused
//! component onto the compositor stack, return a Lua-facing handle
//! whose only method is `close()` (idempotent).
//!
//! The handle holds a shared atomic flag; `close()` flips it. The
//! `LuaWindowComponent` it pushed checks the flag on its next poll
//! tick and pops itself off the stack via `win.close()`.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use mlua::UserData;

/// Shared close flag between the Lua-side handle and the Rust-side
/// component. Cloned at construction; either side flipping it
/// triggers the next poll-tick `win.close()`.
#[derive(Clone, Default)]
pub struct CloseFlag(Arc<AtomicBool>);

impl CloseFlag {
    // Relaxed is sufficient: the flag is the only shared state and
    // there is no companion data whose write must be ordered with
    // respect to the flip.
    pub fn request(&self) {
        self.0.store(true, Ordering::Relaxed);
    }
    pub fn take(&self) -> bool {
        self.0.swap(false, Ordering::Relaxed)
    }
}

/// Lua-facing handle returned by `ttymap.api.window.open(...)`.
/// Idempotent `:close()` — flipping a flipped flag is a no-op.
pub struct WindowHandle {
    flag: CloseFlag,
}

impl WindowHandle {
    /// Build a handle that signals close via the shared `flag`.
    pub fn new(flag: CloseFlag) -> Self {
        Self { flag }
    }
}

impl UserData for WindowHandle {
    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("close", |_, this, _: ()| {
            this.flag.request();
            Ok(())
        });
    }
}

/// Generic wrapper that drains a [`CloseFlag`] on each `poll` tick
/// and forwards every other [`Component`] method to the inner impl.
///
/// Used by `ttymap.api.palette.open` (and any future A-series
/// primitive that wraps a pre-existing `Component`) to add the
/// shared-flag close protocol without duplicating it inside each
/// adapter type. [`LuaWindowComponent`] does the same thing inline
/// in its own `poll` because it owns the flag directly; for adapters
/// that already exist (`PaletteComponent` wrapping a
/// [`LuaPaletteProvider`]) wrapping is the cheaper bridge.
///
/// [`Component`]: crate::frontend::compositor::Component
/// [`LuaWindowComponent`]: super::window_component::LuaWindowComponent
/// [`LuaPaletteProvider`]: super::palette_provider::LuaPaletteProvider
pub struct CloseFlagWrapper<C> {
    inner: C,
    flag: CloseFlag,
}

impl<C> CloseFlagWrapper<C> {
    pub fn new(inner: C, flag: CloseFlag) -> Self {
        Self { inner, flag }
    }
}

impl<C: crate::frontend::compositor::Component> crate::frontend::compositor::Component
    for CloseFlagWrapper<C>
{
    fn handle_event(
        &mut self,
        event: crossterm::event::KeyEvent,
        win: &mut crate::frontend::compositor::window::Window,
    ) {
        self.inner.handle_event(event, win);
    }

    fn render(&self, win: &mut crate::frontend::compositor::window::RenderWindow) {
        self.inner.render(win);
    }

    fn poll(&mut self, win: &mut crate::frontend::compositor::window::Window) {
        self.inner.poll(win);
        if self.flag.take() {
            win.close();
        }
    }

    fn footer_hints(&self) -> Vec<(&'static str, &'static str)> {
        self.inner.footer_hints()
    }

    fn name(&self) -> &'static str {
        self.inner.name()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn close_flips_flag_idempotent() {
        let flag = CloseFlag::default();
        let lua = mlua::Lua::new();
        let ud = lua
            .create_userdata(WindowHandle::new(flag.clone()))
            .unwrap();
        lua.load("local h = ...; h:close(); h:close()")
            .call::<()>(ud)
            .unwrap();
        assert!(flag.take());
        assert!(!flag.take());
    }

    /// [`CloseFlagWrapper::poll`] must call `win.close()` exactly when
    /// the shared flag has been flipped — and nothing extra at any
    /// other time. Mirrors the equivalent test on
    /// [`super::window_component::LuaWindowComponent`].
    #[test]
    fn close_flag_wrapper_polls_close_when_flag_set() {
        use crate::frontend::AppEvent;
        use crate::frontend::compositor::Component;
        use crate::frontend::compositor::Context;
        use crate::frontend::compositor::window::{Window, WindowOps};

        /// Inert inner component — no-op for every method so the
        /// wrapper's behaviour is the only thing under test.
        struct Inert;
        impl Component for Inert {}

        const CTX: Context = Context {
            theme_id: crate::theme::ThemeId::Dark,
            cursor: None,
        };
        let (tx, _rx) = std::sync::mpsc::channel::<AppEvent>();

        let flag = CloseFlag::default();
        let mut wrapped = CloseFlagWrapper::new(Inert, flag.clone());

        // No flip → no close.
        let mut ops = WindowOps::default();
        {
            let mut win = Window::new(&mut ops, &CTX, &tx);
            wrapped.poll(&mut win);
        }
        assert!(!ops.close);

        // Flipped flag → close queued, then idempotent.
        flag.request();
        let mut ops = WindowOps::default();
        {
            let mut win = Window::new(&mut ops, &CTX, &tx);
            wrapped.poll(&mut win);
        }
        assert!(ops.close);

        let mut ops = WindowOps::default();
        {
            let mut win = Window::new(&mut ops, &CTX, &tx);
            wrapped.poll(&mut win);
        }
        assert!(!ops.close);
    }
}
