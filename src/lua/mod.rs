//! Lua runtime scaffold for scripted plugins.
//!
//! Owns the shared [`mlua::Lua`] state. The bridge surface (Component
//! adapter, MapApi, widget descriptors, etc.) lands in submodules as
//! it gets built out per the audit in `docs/lua-bridge-surface.md`.
//!
//! Defaults that are deliberate and not provisional (see audit §13):
//! - **Lua 5.4** via the `vendored` mlua feature: portable, no
//!   system Lua dep, ~1 MB binary growth.
//! - **No sandbox**: ttymap is single-user; trust the plugin author
//!   (the maintainer themself for now).
//! - **Errors are logged, not propagated**: a buggy plugin must not
//!   crash the host. Helpers in this module wrap mlua results with
//!   `log::warn!` + recovery default.

pub mod component;
pub mod host;
pub mod map_api;

pub use component::LuaComponent;

use mlua::Lua;

use crate::compositor::Registrar;

/// Build a fresh Lua state. Sandboxing / standard-library trimming
/// would happen here; for now we hand back the unmodified VM.
pub fn new_lua() -> Lua {
    Lua::new()
}

/// Bundled `hello` plugin source — a tiny demo that doubles as the
/// reference template for future Lua plugin authors.
const HELLO_LUA: &str = include_str!("scripts/hello.lua");

/// Wire the bundled Lua plugins into the registrar. Today that's
/// just `hello`. New bundled plugins go in `scripts/<name>.lua` and
/// register here alongside `hello`.
///
/// Lua failures (parse error, missing fields) are logged and the
/// plugin is silently skipped rather than aborting startup — the
/// host always boots even with a broken Lua plugin.
pub fn register(r: &mut Registrar) {
    register_script("hello", HELLO_LUA, r);
}

fn register_script(name: &'static str, source: &'static str, r: &mut Registrar) {
    // Validate the script up front so a syntax error surfaces as one
    // log line instead of a noisy first-toggle failure.
    if let Err(e) = LuaComponent::from_source(source, name) {
        log::warn!("lua[{}]: failed to load, plugin skipped: {}", name, e);
        return;
    }
    let label = format!("Toggle Lua: {}", name);
    r.add_toggle(label, "", move |_| {
        // Re-load on every toggle so the plugin gets fresh state.
        // If it parsed once at startup it should parse again, but
        // recover gracefully if it doesn't.
        LuaComponent::from_source(source, name).unwrap_or_else(|e| {
            log::warn!("lua[{}]: re-load failed: {}", name, e);
            // Synthesize a minimal placeholder so the toggle still
            // produces a Component. Empty source still parses and
            // exposes a no-op render.
            LuaComponent::from_source("return {}", name).expect("trivial Lua module always loads")
        })
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use mlua::Result;

    #[test]
    fn lua_evaluates_a_basic_expression() {
        let lua = new_lua();
        let n: i64 = lua.load("return 2 + 2").eval().expect("eval 2+2");
        assert_eq!(n, 4);
    }

    #[test]
    fn lua_can_call_a_rust_closure() {
        let lua = new_lua();
        let double = lua
            .create_function(|_, n: i64| Ok(n * 2))
            .expect("create_function");
        lua.globals().set("double", double).expect("set global");
        let n: i64 = lua.load("return double(7)").eval().expect("call");
        assert_eq!(n, 14);
    }

    #[test]
    fn lua_can_return_a_table_to_rust() -> Result<()> {
        let lua = new_lua();
        let table: mlua::Table = lua.load("return { name = 'hi', n = 3 }").eval()?;
        let name: String = table.get("name")?;
        let n: i64 = table.get("n")?;
        assert_eq!(name, "hi");
        assert_eq!(n, 3);
        Ok(())
    }

    #[test]
    fn bundled_hello_script_parses() {
        // The script is in-tree; if this ever fails, the include_str!
        // is pointing at something broken.
        LuaComponent::from_source(HELLO_LUA, "hello").expect("hello.lua should parse");
    }

    #[test]
    fn register_adds_a_palette_entry_for_hello() {
        let mut r = Registrar::default();
        register(&mut r);
        // We don't need to assert the count exactly — just that the
        // bundled hello plugin produced *some* palette entry. The
        // label substring guards against silent regressions.
        let labels: Vec<&str> = r.palette_entries.iter().map(|e| e.label.as_str()).collect();
        assert!(
            labels.iter().any(|l| l.contains("hello")),
            "expected a 'hello' palette entry, got {:?}",
            labels,
        );
    }
}
