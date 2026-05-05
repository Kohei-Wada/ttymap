//! [`Component`] trait — the user-facing extension point for any UI
//! entity that can be pushed onto the compositor stack.
//!
//! Plus the small support types — [`Context`] (read-only app-state
//! snapshot a component sees during a hook) and [`Placement`] (where
//! the component is drawn).

use crossterm::event::KeyEvent;

use super::window::{self, Window};
use crate::theme::ThemeId;

/// Read-only snapshot of app-level context a component may need
/// during a hook. Reached by the component through
/// [`Window::ctx`](super::window::Window::ctx).
#[derive(Debug, Clone, Copy)]
pub struct Context {
    pub theme_id: ThemeId,
    /// Latest mouse cursor position in absolute terminal cells.
    /// `None` until the first mouse event arrives (or always, on
    /// terminals without mouse support). Project to a `LonLat` via
    /// [`MapApi::cursor_ll`](crate::lua::MapApi::cursor_ll)
    /// at paint time.
    #[allow(dead_code)] // plugin-author API; the in-tree reader (info plugin) lands later
    pub cursor: Option<(u16, u16)>,
}

/// Where the component renders. Today only two slots:
///
/// - `Floating`: drawn over the map area, on top of everything.
///   Used for the command palette (the only floating component
///   in tree). Lua plugins are *not* allowed to be Floating —
///   the spec the bridge accepts always lands in `Sidebar`.
/// - `Sidebar`: shares the left sidebar via equal vertical split
///   so multiple sections can live there at once.
///
/// Default is `Floating` because the only Rust-side `Component`
/// impl that doesn't override is the palette, and palette is the
/// canonical floating component.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Placement {
    Floating,
    Sidebar,
}

/// A focus-capable UI entity. Pushed on activation, popped on close.
/// No `is_visible` / `activate` / `deactivate` contract — existence on
/// the stack is the visibility lifecycle.
///
/// nvim-style: the compositor never deduplicates pushes. Pressing an
/// activation key twice produces two instances of the plugin on the
/// stack. Plugins that want toggle behavior implement self-close in
/// their own `handle_key` (return `win.close()` when the activation
/// key fires while focused).
///
/// The event-producing hooks ([`handle_key`](Self::handle_key)
/// and [`poll`](Self::poll)) receive a
/// [`&mut Window`](super::window::Window) and express intent through it
/// (`win.close()`, `win.open(c)`, `win.emit(msg)`, `win.ignore()`).
/// The framework applies those ops atomically after the hook
/// returns, so components cannot break stack / focus invariants
/// regardless of what order they call the methods.
pub trait Component {
    /// Handle a single key event. Call `win.close()` / `open(c)` /
    /// `emit(msg)` / `ignore()` to express what should happen next.
    /// Silence (no `win.*` call) is implicit consumption — the
    /// event is treated as handled but with no state change.
    ///
    /// Default impl is `win.ignore()` — the non-modal "I don't bind
    /// any keys, pass through to the base layer" behaviour. Plugins
    /// that consume keys override this.
    fn handle_key(&mut self, _event: KeyEvent, win: &mut Window) {
        win.ignore();
    }

    /// Paint this component into `win.area()`. Called once per
    /// frame while on the stack; compositor renders bottom-to-top.
    /// `win` carries the ratatui frame, the component's allowed
    /// area, and the current theme — plugins read all three through
    /// `win` so theme does not thread through helper signatures.
    ///
    /// Default impl is no-op — for components that exist only to
    /// hold focus / poll async work, with no sidebar UI of their
    /// own.
    fn render(&self, _win: &mut window::RenderWindow) {}

    /// Where to draw this component. Defaults to
    /// [`Placement::Floating`] (free-floating panel over the
    /// map). Plugins that want to live in the left sidebar
    /// override to [`Placement::Sidebar`].
    fn placement(&self) -> Placement {
        Placement::Floating
    }

    /// Advance async work and surface new messages. Called every tick
    /// on every component on the stack. Use `win.emit(msg)` to
    /// dispatch app-level state changes when a future completes,
    /// and `win.close()` if the component should self-remove.
    fn poll(&mut self, _win: &mut Window) {}

    /// Footer hints shown while this component is on top.
    fn footer_hints(&self) -> Vec<(&'static str, &'static str)> {
        Vec::new()
    }

    /// Short user-facing label shown in the footer when this
    /// component is focused — e.g. `"wiki"`, `"aircraft"`. Defaults
    /// to empty so the bottom layer (or any unlabelled component)
    /// renders no chrome. Plugins return a fixed string token.
    fn name(&self) -> &'static str {
        ""
    }
}
