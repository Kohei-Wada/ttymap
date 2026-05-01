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

use std::sync::mpsc;
use std::time::Duration;

use mlua::{Lua, Table};

use super::handle::{CallOutcome, LuaHandle};
use crate::app::AppMsg;
use crate::compositor::Context;
use crate::geo::LonLat;
use crate::palette::provider::{PaletteAction, PaletteItem, PaletteProvider, SubmitMode};

/// Boxed PaletteProvider that dispatches to a Lua module.
pub struct LuaPaletteProvider {
    /// Bridge plumbing shared with `LuaComponent`. The registered
    /// table here is the `module.palette` sub-table — every method
    /// (filter / items / execute / poll / is_loading) reads from
    /// it.
    handle: LuaHandle,
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
    /// Inbox for `ttymap.map:jump(lon, lat)` calls made from inside
    /// `execute`. Drained right before the action is returned so
    /// the Run-with-Jump path is preserved without exposing the
    /// `AppMsg` enum to Lua.
    jump_rx: mpsc::Receiver<LonLat>,
}

impl LuaPaletteProvider {
    /// Build a palette provider from a `spec` table that was already
    /// constructed in an existing Lua VM (the *setup state* — the VM
    /// that ran the script's top-level `register_*` calls and continues
    /// to run palette / keybind callbacks). Used by
    /// `ttymap.api.palette.open(spec)` where the script builds the spec
    /// inline inside an activation callback rather than at top level.
    ///
    /// Host services (`ttymap.map`, `ttymap.api`, …) are already
    /// installed on `lua` by the prior [`crate::lua::ttymap::install`]
    /// call that produced the setup state.
    ///
    /// **Channel ownership** (per A3's Path 4 decision): `ttymap.map:jump`
    /// inside this provider's `execute` callback hits the **setup
    /// state's** `jump_tx` — drained centrally by [`crate::app::App`]
    /// per frame and emitted as `AppMsg::Map(Action::Jump)`. This
    /// provider's own `jump_rx` is a disconnected channel (sender
    /// dropped immediately) — the in-execute "jump → Run([Jump])" path
    /// is unreachable here, replaced by the App's central drain.
    /// `execute` always collapses to `Close` (modulo the legacy
    /// `jump_rx` priority check kept for forward compatibility).
    pub fn from_spec(lua: Lua, spec: Table, log_tag: &'static str) -> mlua::Result<Self> {
        let prompt: String = spec.get("prompt").unwrap_or_else(|_| ":".to_string());
        let submit_mode = parse_submit_mode(&spec);
        let handle = LuaHandle::new(lua, spec, log_tag)?;

        // Disconnected jump channel — see doc comment above. The
        // sender goes out of scope immediately, so every `try_recv`
        // is `Err` and `action_from_lua` falls through to `Close`.
        // Real jump intent goes through the setup state's `jump_tx`
        // (held by `ttymap.map`) and is drained centrally by App.
        let (_jump_tx, jump_rx) = mpsc::channel();

        Ok(Self {
            handle,
            prompt,
            submit_mode,
            items: Vec::new(),
            jump_rx,
        })
    }

    /// Re-pull `items()` from Lua and cache them. Called after
    /// `filter` and `poll` since both can change the result list.
    fn refresh_items(&mut self) {
        match self.handle.try_call::<_, Vec<Table>>("items", ()) {
            CallOutcome::Ok(raw) => {
                self.items = raw
                    .into_iter()
                    .map(|t| PaletteItem {
                        label: t.get("label").unwrap_or_default(),
                        hint: t.get("hint").unwrap_or_default(),
                    })
                    .collect();
            }
            // Missing items() means the palette never produces rows
            // (rare but legal); error already logged.
            CallOutcome::Missing | CallOutcome::Errored => self.items.clear(),
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
        let _ = self.handle.try_call::<_, ()>("filter", query.to_string());
        self.refresh_items();
    }

    fn items(&self) -> &[PaletteItem] {
        &self.items
    }

    fn execute(&mut self, idx: usize, _ctx: &Context) -> PaletteAction {
        // Lua arrays are 1-indexed; PaletteProvider hands us a
        // 0-based index. Bridge here so the script reads naturally.
        match self
            .handle
            .try_call::<_, mlua::Value>("execute", (idx + 1) as i64)
        {
            CallOutcome::Ok(ret) => self.action_from_lua(ret),
            // Missing handler or runtime error → close. Errors are
            // already logged inside `try_call`.
            CallOutcome::Missing | CallOutcome::Errored => PaletteAction::Close,
        }
    }

    fn submit_mode(&self) -> SubmitMode {
        self.submit_mode
    }

    fn poll(&mut self) {
        let _ = self.handle.try_call::<_, ()>("poll", ());
        self.refresh_items();
    }

    fn is_loading(&self) -> bool {
        match self.handle.try_call::<_, bool>("is_loading", ()) {
            CallOutcome::Ok(b) => b,
            CallOutcome::Missing | CallOutcome::Errored => false,
        }
    }
}

impl LuaPaletteProvider {
    /// Translate a Lua-returned execute() value into a PaletteAction.
    /// Three accepted forms:
    /// - `nil` → Close
    /// - `{ close = true }` → Close
    /// - `ttymap.map:jump(lon, lat)` inside execute → Run([Map(Action::Jump(ll))])
    fn action_from_lua(&self, value: mlua::Value) -> PaletteAction {
        // First check the in-execute jump channel — `ttymap.map:jump` pushes
        // a LonLat that takes priority over any returned table since
        // the script's intent ("jump to this") is unambiguous.
        let mut jumps = Vec::new();
        while let Ok(ll) = self.jump_rx.try_recv() {
            jumps.push(AppMsg::Map(crate::map::Action::Jump(ll)));
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
    use crate::lua::ttymap::{LuaHostShared, install, new_capture_slot};
    use crate::theme::ThemeId;

    fn ctx() -> Context {
        Context {
            center: LonLat { lon: 0.0, lat: 0.0 },
            theme_id: ThemeId::Dark,
            cursor: None,
        }
    }

    /// Build a setup-state Lua VM (`ttymap` global installed) and run
    /// `script` against it, expecting `script` to return the spec
    /// table for `from_spec`. Mirrors how `ttymap.api.palette.open`
    /// receives a spec from a Lua callback at runtime.
    fn build_provider(script: &str) -> LuaPaletteProvider {
        let lua = Lua::new();
        let slot = new_capture_slot();
        let _handles =
            install(&lua, "lua-test", LuaHostShared::empty(), slot).expect("install ttymap");
        let spec: Table = lua.load(script).eval().expect("eval spec");
        LuaPaletteProvider::from_spec(lua, spec, "lua-test").expect("from_spec")
    }

    #[test]
    fn prompt_falls_back_to_colon_when_spec_omits_it() {
        let p = build_provider("return {}");
        assert_eq!(p.prompt(), ":");
    }

    #[test]
    fn prompt_picks_up_spec_value() {
        let p = build_provider(r#"return { prompt = "/" }"#);
        assert_eq!(p.prompt(), "/");
    }

    #[test]
    fn submit_mode_defaults_on_each_key() {
        let p = build_provider("return {}");
        assert!(matches!(p.submit_mode(), SubmitMode::OnEachKey));
    }

    #[test]
    fn submit_mode_string_debounced_uses_default_ms() {
        let p = build_provider(r#"return { submit_mode = "debounced" }"#);
        match p.submit_mode() {
            SubmitMode::Debounced(d) => assert_eq!(d, Duration::from_millis(400)),
            _ => panic!("expected Debounced"),
        }
    }

    #[test]
    fn submit_mode_table_lets_plugin_pick_ms() {
        let p = build_provider(r#"return { submit_mode = { kind = "debounced", ms = 250 } }"#);
        match p.submit_mode() {
            SubmitMode::Debounced(d) => assert_eq!(d, Duration::from_millis(250)),
            _ => panic!("expected Debounced"),
        }
    }

    #[test]
    fn items_round_trip_through_lua() {
        // The spec captures `items_list` as a closure upvalue inside
        // the same Lua VM the provider runs against, so `filter`
        // mutating it is visible to the next `items` call.
        let mut p = build_provider(
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
        );
        p.filter("hi");
        assert_eq!(p.items().len(), 2);
        assert_eq!(p.items()[0].label, "hi a");
        assert_eq!(p.items()[1].hint, "hb");
    }

    #[test]
    fn execute_close_table_returns_close() {
        let mut p =
            build_provider(r#"return { execute = function(_) return { close = true } end }"#);
        assert!(matches!(p.execute(0, &ctx()), PaletteAction::Close));
    }

    #[test]
    fn is_loading_defaults_false() {
        let p = build_provider("return {}");
        assert!(!p.is_loading());
    }

    #[test]
    fn is_loading_reads_spec_function() {
        let p = build_provider(r#"return { is_loading = function() return true end }"#);
        assert!(p.is_loading());
    }
}
