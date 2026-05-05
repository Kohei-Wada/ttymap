//! `Registrar` — collection bucket for everything a Lua plugin
//! declared at script-load time.
//!
//! Each `.lua` file under any runtime layer's `plugin/` directory
//! gets its `register_palette_command` / `register_keybind` /
//! `on_event` calls captured into a [`CapturedRegistration`]. This
//! module's [`build_subsystem`](crate::lua::build_subsystem)
//! consumes those captures and pushes the result into the fields of
//! a single shared [`Registrar`]:
//!
//! - `activations` / `palette_entries` carry compositor primitives
//!   (each holds a `SpawnComponent` factory closure).
//! - `event_bus` carries every `on_event` subscriber for fan-out.
//! - `lua_host_handles` lets the App refresh per-plugin shared
//!   `center` / `zoom` cells before each frame.
//!
//! `Registrar` lives in `lua/` rather than `compositor/` because it
//! is *purely* the Lua side's collection bucket — the compositor
//! itself never names it. Keeping it here keeps `compositor/` a
//! clean focus/modal primitive with no `crate::lua` imports.

use crossterm::event::{KeyCode, KeyModifiers};

use crate::compositor::{Activation, Component, Context, PaletteEntry, SpawnComponent};
use crate::event::EventBus;
use crate::lua::host::LuaHostHandles;

#[derive(Default)]
pub struct Registrar {
    pub activations: Vec<Activation>,
    pub palette_entries: Vec<PaletteEntry>,
    /// Plugin-declared per-frame callbacks. Captured by the Lua
    /// dispatcher when a script calls `ttymap.api.frame.on_tick(fn)`
    /// (zero or more times per script), and ticked once per frame
    /// from `App::run` against the live `MapApi`. The unified
    /// per-frame work mechanism for the nvim-style plugin API.
    pub event_bus: EventBus,
    /// Setup-state [`LuaHostHandles`] for every plugin script: the
    /// App takes ownership of this `Vec` in
    /// [`crate::app::App::new`] and refreshes each handle's shared
    /// view cells once per frame so callbacks running in the setup
    /// state see the live `center` / `zoom`.
    pub lua_host_handles: Vec<LuaHostHandles>,
}

impl Registrar {
    pub fn add_activation(&mut self, a: Activation) {
        self.activations.push(a);
    }
    pub fn add_palette_entry(&mut self, e: PaletteEntry) {
        self.palette_entries.push(e);
    }

    // ── Convenience builders ───────────────────────────────────────────────
    //
    // The methods below accept an `impl Component`-returning closure
    // and box twice internally so each plugin's `register` can drop
    // the `Box::new(move |...| -> Box<dyn Component> { Box::new(...) })`
    // syntactic noise. The struct-literal forms above stay for any
    // plugin that needs full control (e.g. building entries
    // dynamically).

    /// Bind a key to spawn a fresh component on press.
    pub fn bind<F, C>(&mut self, code: KeyCode, modifiers: KeyModifiers, factory: F)
    where
        F: Fn(&Context) -> C + 'static,
        C: Component + 'static,
    {
        self.add_activation(Activation {
            code,
            modifiers,
            spawn: box_component_factory(factory),
        });
    }

    /// Add a palette entry that pushes a fresh component on
    /// selection. Plugins that want toggle behavior implement self-
    /// close in their own `handle_key`.
    pub fn add_palette<F, C>(
        &mut self,
        label: impl Into<String>,
        hint: impl Into<String>,
        name: &'static str,
        factory: F,
    ) where
        F: Fn(&Context) -> C + 'static,
        C: Component + 'static,
    {
        self.add_palette_entry(PaletteEntry {
            label: label.into(),
            hint: hint.into(),
            name,
            spawn: box_component_factory(factory),
        });
    }
}

/// Wrap an `impl Component`-returning closure in the double-Box that
/// the registrar's collections store. Lifts the `Box::new(move |ctx|
/// Box::new(factory(ctx)) as Box<dyn Component>)` boilerplate out of
/// every `add_*` method so the next builder doesn't have to remember
/// the exact dance.
fn box_component_factory<F, C>(factory: F) -> SpawnComponent
where
    F: Fn(&Context) -> C + 'static,
    C: Component + 'static,
{
    Box::new(move |ctx| Some(Box::new(factory(ctx)) as Box<dyn Component>))
}
