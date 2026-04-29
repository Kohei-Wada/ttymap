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

#[allow(unused_imports)] // re-export becomes load-bearing once a plugin registers
pub use component::LuaComponent;

use mlua::Lua;

/// Build a fresh Lua state. Sandboxing / standard-library trimming
/// would happen here; for now we hand back the unmodified VM.
#[allow(dead_code)] // exercised by component.rs tests; first non-test caller lands when LuaComponent is registered.
pub fn new_lua() -> Lua {
    Lua::new()
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
}
