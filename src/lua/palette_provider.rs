//! [`LuaPaletteProvider`] — adapter that lets a Lua script implement
//! [`PaletteProvider`].
//!
//! Same shape as [`super::component::LuaComponent`] but for the
//! palette's universal-picker trait instead of the compositor
//! [`Component`] trait. Used by the search plugin's Lua port:
//! Nominatim's debounced query/result pipeline + Enter→Jump.
//!
//! Errors in any callback are logged + recovered (empty results,
//! Close action) to keep a buggy plugin from crashing the host.

use std::sync::{Arc, mpsc};
use std::time::Duration;

use mlua::{Lua, RegistryKey, Table};

use crate::app::AppMsg;
use crate::compositor::Context;
use crate::geo::LonLat;
use crate::lua::host::LuaHostShared;
use crate::palette::provider::{PaletteAction, PaletteItem, PaletteProvider, SubmitMode};

/// Boxed PaletteProvider that dispatches to a Lua module.
pub struct LuaPaletteProvider {
    lua: Lua,
    module: RegistryKey,
    /// Cached `prompt` string read once at construction so
    /// [`PaletteProvider::prompt`] can hand back `&str` without
    /// running Lua per call.
    prompt: String,
    /// Cached submit mode read once at construction.
    submit_mode: SubmitMode,
    /// Cached items rebuilt by `filter` and `poll`. The trait method
    /// `items()` returns `&[PaletteItem]` so we keep a local copy
    /// rather than round-tripping into Lua per call.
    items: Vec<PaletteItem>,
    /// Inbox for `host:jump(lon, lat)` calls made from inside
    /// `execute`. Drained right before the action is returned so
    /// the Run-with-Jump path is preserved without exposing the
    /// `AppMsg` enum to Lua.
    jump_rx: mpsc::Receiver<LonLat>,
}

impl LuaPaletteProvider {
    /// Convenience for tests / standalone use. Production callers
    /// thread the live [`LuaHostShared`] through `from_source_full`.
    #[cfg(test)]
    pub fn from_source(source: &'static str, chunk_name: &str) -> mlua::Result<Box<Self>> {
        Self::from_source_full(source, chunk_name, super::host::LuaHostShared::empty())
    }

    pub fn from_source_full(
        source: &'static str,
        chunk_name: &str,
        shared: Arc<LuaHostShared>,
    ) -> mlua::Result<Box<Self>> {
        let lua = super::new_lua();

        // Same `host` global the Component bridge uses. We only need
        // the jump channel here; close / export aren't meaningful for
        // a palette provider (the palette closes itself based on
        // PaletteAction).
        let (host, handles) = super::host::LuaHost::new("lua-palette", shared);
        let host_ud = lua.create_userdata(host)?;
        lua.globals().set("host", host_ud)?;

        let module: Table = lua.load(source).set_name(chunk_name).eval()?;

        let prompt: String = module.get("prompt").unwrap_or_else(|_| ":".to_string());
        let submit_mode = parse_submit_mode(&module);
        let module = lua.create_registry_value(module)?;

        Ok(Box::new(Self {
            lua,
            module,
            prompt,
            submit_mode,
            items: Vec::new(),
            jump_rx: handles.jump_rx,
        }))
    }

    fn module(&self) -> mlua::Result<Table> {
        self.lua.registry_value(&self.module)
    }

    /// Re-pull `items()` from Lua and cache them. Called after
    /// `filter` and `poll` since both can change the result list.
    fn refresh_items(&mut self) {
        let result: mlua::Result<Vec<PaletteItem>> = (|| {
            let items_fn: Option<mlua::Function> = self.module()?.get("items").ok();
            let Some(items_fn) = items_fn else {
                return Ok(Vec::new());
            };
            let raw: Vec<Table> = items_fn.call(())?;
            Ok(raw
                .into_iter()
                .map(|t| PaletteItem {
                    label: t.get("label").unwrap_or_default(),
                    hint: t.get("hint").unwrap_or_default(),
                })
                .collect())
        })();
        match result {
            Ok(items) => self.items = items,
            Err(e) => {
                log::warn!("lua-palette: items() failed: {}", e);
                self.items.clear();
            }
        }
    }
}

/// Parse `module.submit_mode` — defaults to `OnEachKey` so a plugin
/// that omits it gets the same shape as the built-in command picker.
/// Accepted forms:
/// - `nil` (missing) → OnEachKey
/// - `"on_each_key"` → OnEachKey
/// - `"debounced"` → Debounced(400ms) (sane default for Nominatim)
/// - `{ kind = "debounced", ms = 400 }` → Debounced(custom)
fn parse_submit_mode(module: &Table) -> SubmitMode {
    let val = module.get::<mlua::Value>("submit_mode").ok();
    match val {
        Some(mlua::Value::Nil) | None => SubmitMode::OnEachKey,
        Some(mlua::Value::String(s)) => match s.to_str().as_deref() {
            Ok("debounced") => SubmitMode::Debounced(Duration::from_millis(400)),
            _ => SubmitMode::OnEachKey,
        },
        Some(mlua::Value::Table(t)) => {
            let kind: String = t.get("kind").unwrap_or_default();
            if kind == "debounced" {
                let ms: u64 = t.get("ms").unwrap_or(400);
                SubmitMode::Debounced(Duration::from_millis(ms))
            } else {
                SubmitMode::OnEachKey
            }
        }
        _ => SubmitMode::OnEachKey,
    }
}

impl PaletteProvider for LuaPaletteProvider {
    fn prompt(&self) -> &str {
        &self.prompt
    }

    fn filter(&mut self, query: &str) {
        let result: mlua::Result<()> = (|| {
            let module = self.module()?;
            let f: Option<mlua::Function> = module.get("filter").ok();
            let Some(f) = f else {
                return Ok(());
            };
            f.call::<()>(query.to_string())
        })();
        if let Err(e) = result {
            log::warn!("lua-palette: filter() failed: {}", e);
        }
        self.refresh_items();
    }

    fn items(&self) -> &[PaletteItem] {
        &self.items
    }

    fn execute(&mut self, idx: usize, _ctx: &Context) -> PaletteAction {
        let result: mlua::Result<PaletteAction> = (|| {
            let module = self.module()?;
            let f: Option<mlua::Function> = module.get("execute").ok();
            let Some(f) = f else {
                return Ok(PaletteAction::Close);
            };
            // Lua arrays are 1-indexed; PaletteProvider hands us a
            // 0-based index. Bridge here so the script reads
            // naturally.
            let ret: mlua::Value = f.call((idx + 1) as i64)?;
            Ok(self.action_from_lua(ret))
        })();
        match result {
            Ok(action) => action,
            Err(e) => {
                log::warn!("lua-palette: execute() failed: {}", e);
                PaletteAction::Close
            }
        }
    }

    fn submit_mode(&self) -> SubmitMode {
        self.submit_mode
    }

    fn poll(&mut self) {
        let result: mlua::Result<()> = (|| {
            let module = self.module()?;
            let f: Option<mlua::Function> = module.get("poll").ok();
            let Some(f) = f else {
                return Ok(());
            };
            f.call::<()>(())
        })();
        if let Err(e) = result {
            log::warn!("lua-palette: poll() failed: {}", e);
        }
        self.refresh_items();
    }

    fn is_loading(&self) -> bool {
        let result: mlua::Result<bool> = (|| {
            let module = self.module()?;
            let f: Option<mlua::Function> = module.get("is_loading").ok();
            match f {
                Some(f) => f.call(()),
                None => Ok(false),
            }
        })();
        result.unwrap_or(false)
    }
}

impl LuaPaletteProvider {
    /// Translate a Lua-returned execute() value into a PaletteAction.
    /// Three accepted forms:
    /// - `nil` → Close
    /// - `{ close = true }` → Close
    /// - host:jump(lon, lat) inside execute → Run([Jump(ll)])
    fn action_from_lua(&self, value: mlua::Value) -> PaletteAction {
        // First check the in-execute jump channel — host:jump pushes
        // a LonLat that takes priority over any returned table since
        // the script's intent ("jump to this") is unambiguous.
        let mut jumps = Vec::new();
        while let Ok(ll) = self.jump_rx.try_recv() {
            jumps.push(AppMsg::Jump(ll));
        }
        if !jumps.is_empty() {
            return PaletteAction::Run(jumps);
        }
        // No jump pending: every other return value (nil, `{close=true}`,
        // anything malformed) collapses to Close. Future variants can
        // branch in here if a script ever needs Push/Toggle/SwitchProvider.
        let _ = value;
        PaletteAction::Close
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::ThemeId;

    fn ctx() -> Context {
        Context {
            center: LonLat { lon: 0.0, lat: 0.0 },
            theme_id: ThemeId::Dark,
            cursor: None,
        }
    }

    #[test]
    fn prompt_falls_back_to_colon_when_module_omits_it() {
        let p = LuaPaletteProvider::from_source("return {}", "anon").expect("load");
        assert_eq!(p.prompt(), ":");
    }

    #[test]
    fn prompt_picks_up_module_value() {
        let p =
            LuaPaletteProvider::from_source(r#"return { prompt = "/" }"#, "named").expect("load");
        assert_eq!(p.prompt(), "/");
    }

    #[test]
    fn submit_mode_defaults_on_each_key() {
        let p = LuaPaletteProvider::from_source("return {}", "anon").expect("load");
        assert!(matches!(p.submit_mode(), SubmitMode::OnEachKey));
    }

    #[test]
    fn submit_mode_string_debounced_uses_default_ms() {
        let p = LuaPaletteProvider::from_source(r#"return { submit_mode = "debounced" }"#, "anon")
            .expect("load");
        match p.submit_mode() {
            SubmitMode::Debounced(d) => assert_eq!(d, Duration::from_millis(400)),
            _ => panic!("expected Debounced"),
        }
    }

    #[test]
    fn submit_mode_table_lets_plugin_pick_ms() {
        let p = LuaPaletteProvider::from_source(
            r#"return { submit_mode = { kind = "debounced", ms = 250 } }"#,
            "anon",
        )
        .expect("load");
        match p.submit_mode() {
            SubmitMode::Debounced(d) => assert_eq!(d, Duration::from_millis(250)),
            _ => panic!("expected Debounced"),
        }
    }

    #[test]
    fn items_round_trip_through_lua() {
        let mut p = LuaPaletteProvider::from_source(
            r#"
            local items_list = {}
            return {
                filter = function(q)
                    items_list = {}
                    if q ~= "" then
                        table.insert(items_list, { label = q .. " a", hint = "ha" })
                        table.insert(items_list, { label = q .. " b", hint = "hb" })
                    end
                end,
                items = function() return items_list end,
            }
            "#,
            "round-trip",
        )
        .expect("load");
        p.filter("hi");
        assert_eq!(p.items().len(), 2);
        assert_eq!(p.items()[0].label, "hi a");
        assert_eq!(p.items()[1].hint, "hb");
    }

    #[test]
    fn execute_jump_returns_run_with_jump() {
        let mut p = LuaPaletteProvider::from_source(
            r#"
            return {
                execute = function(idx)
                    host:jump(139.7, 35.7)
                    return nil
                end,
            }
            "#,
            "exec-jump",
        )
        .expect("load");
        match p.execute(0, &ctx()) {
            PaletteAction::Run(msgs) => {
                assert_eq!(msgs.len(), 1);
                assert!(matches!(msgs[0], AppMsg::Jump(_)));
            }
            _ => panic!("expected Run([Jump])"),
        }
    }

    #[test]
    fn execute_close_table_returns_close() {
        let mut p = LuaPaletteProvider::from_source(
            r#"return { execute = function(_) return { close = true } end }"#,
            "exec-close",
        )
        .expect("load");
        assert!(matches!(p.execute(0, &ctx()), PaletteAction::Close));
    }

    #[test]
    fn is_loading_defaults_false() {
        let p = LuaPaletteProvider::from_source("return {}", "anon").expect("load");
        assert!(!p.is_loading());
    }

    #[test]
    fn is_loading_reads_module_function() {
        let p = LuaPaletteProvider::from_source(
            r#"return { is_loading = function() return true end }"#,
            "loading",
        )
        .expect("load");
        assert!(p.is_loading());
    }
}
