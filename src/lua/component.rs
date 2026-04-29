//! [`LuaComponent`] — adapter that lets a Lua script implement the
//! [`Component`] trait.
//!
//! The script is expected to return a Lua table (its "module"). The
//! adapter caches `name` at construction (so it can satisfy the
//! `&'static str` signature on [`Component::name`] by leaking) and
//! dispatches `render` back into Lua every frame.
//!
//! Per audit §13: errors are logged and recovered, never propagated —
//! a crashing Lua plugin must not take the host down. Failed lookups
//! yield empty render output and a warning in the log.
//!
//! Scope of this PR: `name`, `render` (text lines wrapped in a
//! framed Paragraph). Key handling, paint_on_map, poll, and the
//! richer widget descriptors land in follow-ups.

use mlua::{Lua, RegistryKey, Table};

use crate::compositor::Component;
use crate::compositor::window::RenderWindow;
use crate::widget::{self, Line, Span, StyleKind};

/// A Component implemented in Lua.
#[allow(dead_code)] // first registrar caller lands when the hello.lua plugin is wired into build_registrar
pub struct LuaComponent {
    lua: Lua,
    /// Registry handle for the script's module table — re-fetched
    /// every dispatch via `lua.registry_value::<Table>(&self.module)`.
    module: RegistryKey,
    /// Cached at construction so [`Component::name`] can return
    /// `&'static str`. Leaked once per component; total cost is a
    /// few dozen bytes for the lifetime of the program.
    name: &'static str,
}

#[allow(dead_code)] // mirrors the LuaComponent struct attribute; same reason
impl LuaComponent {
    /// Load `source` as a Lua chunk and capture its returned table as
    /// the component's module.
    ///
    /// `chunk_name` is used in Lua error messages — pass the source
    /// file name (or any identifier) so backtraces are readable.
    pub fn from_source(source: &str, chunk_name: &str) -> mlua::Result<Self> {
        let lua = super::new_lua();
        let module: Table = lua.load(source).set_name(chunk_name).eval()?;
        let raw_name: String = module.get("name").unwrap_or_else(|_| "lua".to_string());
        let name: &'static str = Box::leak(raw_name.into_boxed_str());
        let module = lua.create_registry_value(module)?;
        Ok(Self { lua, module, name })
    }

    /// Pull the `render()` lines from the Lua module. Returns an
    /// empty vec on any error (with a warning logged).
    fn render_lines(&self) -> Vec<String> {
        let result: mlua::Result<Vec<String>> = (|| {
            let module: Table = self.lua.registry_value(&self.module)?;
            let render: mlua::Function = module.get("render")?;
            render.call(())
        })();
        match result {
            Ok(lines) => lines,
            Err(e) => {
                log::warn!("lua[{}]: render() failed: {}", self.name, e);
                Vec::new()
            }
        }
    }
}

impl Component for LuaComponent {
    fn render(&self, win: &mut RenderWindow) {
        let area = win.area();
        let body = win.style(StyleKind::Body);
        let lines: Vec<Line> = self
            .render_lines()
            .into_iter()
            .map(|s| Line::from_span(Span::styled(s, body)))
            .collect();
        let paragraph = widget::Paragraph {
            lines,
            style: body,
            framed_title: Some(self.name.to_string()),
            ..Default::default()
        };
        win.paragraph(paragraph, area);
    }

    fn name(&self) -> &'static str {
        self.name
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_falls_back_when_module_omits_it() {
        // Module with no `name` field — adapter substitutes "lua".
        let c = LuaComponent::from_source("return {}", "anon").expect("load");
        assert_eq!(c.name(), "lua");
    }

    #[test]
    fn name_is_picked_up_from_module() {
        let c = LuaComponent::from_source(
            r#"return { name = "hello", render = function() return {} end }"#,
            "named",
        )
        .expect("load");
        assert_eq!(c.name(), "hello");
    }

    #[test]
    fn render_lines_round_trip_through_lua() {
        let c = LuaComponent::from_source(
            r#"return {
                name = "demo",
                render = function() return { "alpha", "beta", "gamma" } end,
            }"#,
            "demo",
        )
        .expect("load");
        assert_eq!(c.render_lines(), vec!["alpha", "beta", "gamma"]);
    }

    #[test]
    fn render_lines_recovers_when_lua_throws() {
        let c = LuaComponent::from_source(
            r#"return {
                name = "broken",
                render = function() error("kaboom") end,
            }"#,
            "broken",
        )
        .expect("load");
        // Should not panic — error is logged, we get an empty result.
        assert_eq!(c.render_lines(), Vec::<String>::new());
    }

    #[test]
    fn render_lines_recovers_when_field_is_missing() {
        let c = LuaComponent::from_source(r#"return { name = "noop" }"#, "noop").expect("load");
        // No `render` key → graceful fallback.
        assert_eq!(c.render_lines(), Vec::<String>::new());
    }

    #[test]
    fn loading_invalid_lua_returns_error() {
        let err = LuaComponent::from_source("this is not lua syntax !", "bad");
        assert!(err.is_err());
    }
}
