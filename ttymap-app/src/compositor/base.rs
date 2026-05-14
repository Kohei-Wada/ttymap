//! [`BaseLayer`] — the bottom-layer compositor component.
//!
//! Implemented as a [`Component`] that always sits at index 0 of the
//! [`Compositor`](super::Compositor) stack. Handles everything that
//! applies when no modal above it has claimed the key:
//!
//! - **Keymap fallback**: resolves `h/j/k/l/q/0/+/-/…` via [`KeyMap`]
//! - **Activation dispatch**: `:` / `/` / `i` / `?` (and any future
//!   plugin activation key) looked up in an [`Activation`] table.
//!   When the base layer sees an activation key it calls
//!   `win.open(spawn(ctx))` with the freshly-spawned component.
//! - **`gg` multi-key sequence**: tracks `pending_g` and emits
//!   `ZoomToWorld` on the second `g`.
//!
//! Tab / Shift-Tab focus cycling is **not** handled here — it's
//! intercepted by the compositor before any component sees it, so
//! BaseLayer doesn't need to know about it.
//!
//! Because it's always at the very bottom of the stack, its
//! `handle_key` only runs when the focused (possibly higher)
//! component called `win.ignore()` — exactly the old
//! "pass through to background" cases.
//!
//! The base layer renders nothing (the map comes from the render
//! thread's `MapFrame`, drawn by `App` separately from the
//! compositor).

use std::rc::Rc;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::window::{RenderWindow, Window};
use super::{Activation, ActivationIndex, Component, SpawnComponent};
use crate::UserCommand;
use crate::input::keymap::KeyMap;
use ttymap_engine::map::MapAction;

pub struct BaseLayer {
    keymap: KeyMap,
    /// Built-in activations BaseLayer dispatches alongside plugin
    /// activations. Today this is just the `:` palette opener
    /// pushed by [`crate::palette::install`]. Walked first on
    /// keypress, so a plugin can't accidentally shadow `:`.
    builtin_activations: Vec<Activation>,
    /// Read-only view onto whatever store holds plugin activations.
    /// Today backed by the Lua-side registry through
    /// [`crate::lua::LuaActivationIndex`]; BaseLayer itself stays
    /// unaware that Lua is involved.
    activations: Rc<dyn ActivationIndex>,
    /// Lua-supplied footer hints harvested at startup. Static for
    /// the program lifetime — adding / removing entries does not
    /// refresh this list.
    footer_hints: Vec<(&'static str, &'static str)>,
    /// First-`g` flag of the `gg` sequence. Lives here (not in
    /// `KeyMap`) because multi-key sequencing is a base-layer
    /// concern; the keymap itself is a stateless lookup table.
    pending_g: bool,
}

impl BaseLayer {
    pub fn new(
        keymap: KeyMap,
        builtin_activations: Vec<Activation>,
        activations: Rc<dyn ActivationIndex>,
        footer_hints: Vec<(&'static str, &'static str)>,
    ) -> Self {
        Self {
            keymap,
            builtin_activations,
            activations,
            footer_hints,
            pending_g: false,
        }
    }

    /// Look up the spawn factory for a key. Built-ins win over plugin
    /// entries (so `:` always opens the palette regardless of what
    /// plugins did). The trait returns an already-cloned
    /// `SpawnComponent` (cheap `Rc` bump), so invoking it cannot
    /// re-enter whatever borrow the index implementation took.
    fn lookup_spawn(&self, code: KeyCode, modifiers: KeyModifiers) -> Option<SpawnComponent> {
        let clean = modifiers & !KeyModifiers::SHIFT;
        for a in &self.builtin_activations {
            if a.code == code && a.modifiers == clean {
                return Some(a.spawn.clone());
            }
        }
        self.activations.find_spawn(code, clean)
    }

    /// Advance the `gg` state machine and resolve via the keymap.
    /// Called unconditionally so any non-`g` keypress resets
    /// `pending_g`.
    fn resolve_keymap(&mut self, code: KeyCode, modifiers: KeyModifiers) -> Option<UserCommand> {
        if code == KeyCode::Char('g') && modifiers == KeyModifiers::NONE {
            if self.pending_g {
                self.pending_g = false;
                return Some(UserCommand::Map(MapAction::ZoomToWorld));
            }
            self.pending_g = true;
            return None;
        }
        self.pending_g = false;
        self.keymap.resolve(code, modifiers)
    }
}

impl Component for BaseLayer {
    fn handle_key(&mut self, event: KeyEvent, win: &mut Window) {
        let KeyEvent {
            code, modifiers, ..
        } = event;

        // Always advance the gg state first — vim semantics: any
        // non-`g` key (including `:`, activation keys) resets
        // `pending_g`. Tab is already filtered out by the compositor
        // above, so it never reaches here.
        let keymap_msg = self.resolve_keymap(code, modifiers);

        // Activation keys: clone the spawn factory out (cheap Rc
        // bump), then invoke. Holding only the cloned factory
        // means the plugin's own callback can mutably borrow the
        // registry (e.g. via `KeybindHandle:remove()`) without
        // tripping a `RefCell` re-entry panic. Some factories (Lua
        // plugins behind a `register_keybind` callback) may decline
        // by returning `None`; the key is still consumed so it
        // doesn't fall through to the keymap.
        if let Some(spawn) = self.lookup_spawn(code, modifiers) {
            if let Some(new_component) = spawn(win.ctx()) {
                win.open(new_component);
            }
            return;
        }

        if let Some(msg) = keymap_msg {
            win.emit(msg);
        }

        // First `g` of `gg` and unrecognised keys land here. Base
        // layer has nothing below it, so we implicitly consume
        // (no `win.ignore()`).
    }

    fn render(&self, _win: &mut RenderWindow) {
        // Bottom layer draws nothing — the map is painted by `App`
        // separately from the compositor (see `App::run`).
    }

    fn footer_hints(&self) -> Vec<(&'static str, &'static str)> {
        // Core keymap shortcuts that always apply on the map. Plugin-
        // specific bindings (search / wiki / help / …) are appended
        // from the live registrar so the footer reflects what's
        // actually loaded — disabling or rebinding a plugin updates
        // the footer for free.
        let mut hints: Vec<(&'static str, &'static str)> =
            vec![("hjkl", "pan"), ("a/z", "zoom"), (":", "cmd")];
        hints.extend(self.footer_hints.iter().copied());
        hints.push(("q", "quit"));
        hints
    }

    fn name(&self) -> &'static str {
        "map"
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::super::window::WindowOps;
    use super::super::{CardId, Context};
    use super::*;
    use crate::theme::ThemeId;

    const NONE: KeyModifiers = KeyModifiers::NONE;
    const CTX: Context = Context {
        theme_id: ThemeId::Dark,
        cursor: None,
    };

    /// In-test [`ActivationIndex`] backed by a `Vec`. Keeps the
    /// compositor's tests free of any Lua dependency — the registry
    /// remove/round-trip exercise here is structural (does the trait
    /// surface it?), not Lua-specific.
    #[derive(Default)]
    struct MockIndex {
        activations: RefCell<Vec<(u64, Activation)>>,
    }

    impl MockIndex {
        fn add(&self, id: u64, a: Activation) {
            self.activations.borrow_mut().push((id, a));
        }

        fn remove(&self, id: u64) -> bool {
            let mut subs = self.activations.borrow_mut();
            let before = subs.len();
            subs.retain(|(i, _)| *i != id);
            before != subs.len()
        }
    }

    impl ActivationIndex for MockIndex {
        fn find_spawn(&self, code: KeyCode, modifiers: KeyModifiers) -> Option<SpawnComponent> {
            self.activations.borrow().iter().find_map(|(_, a)| {
                if a.code == code && a.modifiers == modifiers {
                    Some(Rc::clone(&a.spawn))
                } else {
                    None
                }
            })
        }
    }

    fn bg() -> BaseLayer {
        BaseLayer::new(
            KeyMap::default(),
            Vec::new(),
            Rc::new(MockIndex::default()),
            Vec::new(),
        )
    }

    /// Dispatch a key into `bg`. Intent emissions and stack ops both
    /// land on the returned [`WindowOps`]; tests inspect via the
    /// `closed()` / `pushed()` / `intents()` helpers.
    fn dispatch(bg: &mut BaseLayer, code: KeyCode) -> WindowOps {
        let mut ops = WindowOps::default();
        {
            let mut win = Window::new(&mut ops, &CTX, CardId::next());
            bg.handle_key(KeyEvent::new(code, NONE), &mut win);
        }
        ops
    }

    #[test]
    fn gg_produces_zoom_to_world_on_second_g() {
        let mut bg = bg();
        // 1st g: nothing fires, pending_g latched.
        let ops = dispatch(&mut bg, KeyCode::Char('g'));
        assert!(ops.intents().is_empty());
        assert!(!ops.closed());
        assert!(!ops.pushed());
        // 2nd g: ZoomToWorld.
        let ops = dispatch(&mut bg, KeyCode::Char('g'));
        assert_eq!(
            ops.intents(),
            vec![UserCommand::Map(MapAction::ZoomToWorld)]
        );
    }

    #[test]
    fn gg_sequence_broken_by_other_key() {
        let mut bg = bg();
        dispatch(&mut bg, KeyCode::Char('g'));
        dispatch(&mut bg, KeyCode::Char('h')); // breaks
        // Now pending_g was reset; this g latches afresh, doesn't fire.
        let ops = dispatch(&mut bg, KeyCode::Char('g'));
        assert!(ops.intents().is_empty());
    }

    #[test]
    fn keybind_removed_from_index_no_longer_dispatches() {
        // Index-driven dispatch round-trip: register an activation
        // for `Z`, confirm the dispatch fires (factory returns Some
        // so a component would be pushed), then remove the activation
        // by id and confirm the next `Z` press is a no-op. Uses the
        // in-test `MockIndex` rather than the Lua-backed wrapper —
        // the compositor sees only the trait surface.
        use super::super::Component;
        use std::cell::Cell;

        let index = Rc::new(MockIndex::default());
        let fired = Rc::new(Cell::new(0_u32));

        struct DummyComponent;
        impl Component for DummyComponent {
            fn handle_key(&mut self, _: KeyEvent, _: &mut Window) {}
            fn render(&self, _: &mut RenderWindow) {}
            fn name(&self) -> &'static str {
                "dummy"
            }
        }

        let id = 42;
        let fired_for_factory = fired.clone();
        index.add(
            id,
            Activation {
                code: KeyCode::Char('Z'),
                modifiers: KeyModifiers::NONE,
                spawn: Rc::new(move |_| -> Option<Box<dyn Component>> {
                    fired_for_factory.set(fired_for_factory.get() + 1);
                    Some(Box::new(DummyComponent))
                }),
            },
        );

        let mut bg = BaseLayer::new(
            KeyMap::default(),
            Vec::new(),
            index.clone() as Rc<dyn ActivationIndex>,
            Vec::new(),
        );

        let ops = dispatch(&mut bg, KeyCode::Char('Z'));
        assert!(ops.pushed(), "registered keybind should push a component");
        assert_eq!(fired.get(), 1);

        // Drop the activation by id — simulating
        // KeybindHandle:remove() from Lua via the index trait.
        assert!(index.remove(id));

        let ops = dispatch(&mut bg, KeyCode::Char('Z'));
        assert!(
            !ops.pushed(),
            "removed keybind must not dispatch (factory must not fire)",
        );
        assert_eq!(fired.get(), 1, "factory must not have fired again");
    }
}
