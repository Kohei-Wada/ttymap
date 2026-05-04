//! [`LuaPaletteProvider`] — adapter that lets a Lua script implement
//! [`PaletteProvider`].
//!
//! Same shape as [`super::card_component::LuaCardComponent`] but
//! for the palette's universal-picker trait instead of the compositor
//! [`Component`] trait. Used by the search plugin's Lua port:
//! Nominatim's debounced query/result pipeline + Enter→Jump.
//!
//! Errors in any callback are logged + recovered (empty results,
//! Close action) to keep a buggy plugin from crashing the host.

use std::time::Duration;

use mlua::{Lua, Table};

use super::handle::{CallOutcome, LuaBridgeHandle};
use crate::frontend::compositor::Context;
use crate::frontend::palette::PaletteAction;
use crate::frontend::palette::provider::{PaletteItem, PaletteProvider, SubmitMode};

/// Boxed PaletteProvider that dispatches to a Lua module.
pub struct LuaPaletteProvider {
    /// Bridge plumbing shared with `LuaCardComponent`. The registered
    /// table here is the `module.palette` sub-table — every method
    /// (filter / items / execute / poll / is_loading) reads from
    /// it.
    handle: LuaBridgeHandle,
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
    /// installed on `lua` by the prior [`crate::lua::api::install`]
    /// call that produced the setup state. `ttymap.map:jump` inside
    /// this provider's `execute` callback hits the setup state's
    /// shared `intent_tx` — drained centrally by [`crate::frontend::App`]
    /// per frame and dispatched as `UserIntent::Map(MapAction::Jump)`.
    /// `execute` returns purely structural [`PaletteAction`] variants
    /// (`Close`, future `Push` / `SwitchProvider`); map intents leave
    /// through the host channel, not the action return value.
    pub fn from_spec(lua: Lua, spec: Table, log_tag: &'static str) -> mlua::Result<Self> {
        let prompt: String = spec.get("prompt").unwrap_or_else(|_| ":".to_string());
        let submit_mode = parse_submit_mode(&spec);
        let handle = LuaBridgeHandle::new(lua, spec, log_tag)?;

        Ok(Self {
            handle,
            prompt,
            submit_mode,
            items: Vec::new(),
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
/// - `"on_enter"` → OnEnter (filter only fires on Enter+empty)
/// - `{ kind = "debounced", ms = 400 }` → Debounced(custom)
/// - `{ kind = "on_enter" }` → OnEnter
fn parse_submit_mode(module: &Table) -> SubmitMode {
    let val = module.get::<mlua::Value>("submit_mode").ok();
    match val {
        Some(mlua::Value::Nil) | None => SubmitMode::OnEachKey,
        Some(mlua::Value::String(s)) => match s.to_str().as_deref() {
            Ok("debounced") => SubmitMode::Debounced(Duration::from_millis(400)),
            Ok("on_enter") => SubmitMode::OnEnter,
            _ => SubmitMode::OnEachKey,
        },
        Some(mlua::Value::Table(t)) => {
            let kind: String = t.get("kind").unwrap_or_default();
            match kind.as_str() {
                "debounced" => {
                    let ms: u64 = t.get("ms").unwrap_or(400);
                    SubmitMode::Debounced(Duration::from_millis(ms))
                }
                "on_enter" => SubmitMode::OnEnter,
                _ => SubmitMode::OnEachKey,
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

    fn cancel(&mut self) -> PaletteAction {
        // Mirrors `execute` for the Esc / Enter-on-empty paths so a
        // Lua plugin can react to non-confirm closes (clear a handle,
        // log, …). Missing callback is normal — most providers don't
        // need a hook — so it falls through to the default Close.
        match self.handle.try_call::<_, mlua::Value>("cancel", ()) {
            CallOutcome::Ok(ret) => self.action_from_lua(ret),
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
    /// Translate a Lua-returned execute() / cancel() value into a
    /// [`PaletteAction`]. Recognised table shapes:
    ///
    /// - `nil` / `{ close = true }` / unrecognised → `Close`
    /// - `{ switch = spec_table }` → `SwitchProvider` with a new
    ///   [`LuaPaletteProvider`] built from `spec_table` (same shape as
    ///   the spec passed to `ttymap.api.palette.open`). Lets a Lua
    ///   plugin do an in-place sub-mode transition (palette stays open,
    ///   prompt + filter + items swap) without close-then-open
    ///   stacking. Invalid spec → warn + `Close`.
    ///
    /// Map intents (`ttymap.map:jump` inside `execute`) take a
    /// different path: they push onto the setup state's shared
    /// `intent_tx`, which the App drains centrally and dispatches.
    /// `Run([UserIntent::*])` therefore has no return-value form — Lua
    /// emits map intents through the host channel, not the action.
    /// `Push` similarly is reachable today via `ttymap.api.card.open`
    /// inside the callback (1 frame of layout latency vs. an in-band
    /// return).
    fn action_from_lua(&self, value: mlua::Value) -> PaletteAction {
        let mlua::Value::Table(t) = &value else {
            return PaletteAction::Close;
        };
        if let Ok(mlua::Value::Table(spec)) = t.get::<mlua::Value>("switch") {
            return match LuaPaletteProvider::from_spec(
                self.handle.lua().clone(),
                spec,
                self.handle.log_tag(),
            ) {
                Ok(provider) => PaletteAction::SwitchProvider(Box::new(provider)),
                Err(e) => {
                    log::warn!(
                        "lua[{}]: palette execute returned invalid switch spec, closing: {}",
                        self.handle.log_tag(),
                        e
                    );
                    PaletteAction::Close
                }
            };
        }
        PaletteAction::Close
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lua::api::{LuaHostShared, install, new_capture_slot};
    use crate::theme::ThemeId;

    fn ctx() -> Context {
        Context {
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
        let _handles = install(
            &lua,
            "lua-test",
            LuaHostShared::empty(),
            slot,
            crate::lua::op::new_ops_buffer(),
        )
        .expect("install ttymap");
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
    fn submit_mode_string_on_enter() {
        let p = build_provider(r#"return { submit_mode = "on_enter" }"#);
        assert!(matches!(p.submit_mode(), SubmitMode::OnEnter));
    }

    #[test]
    fn submit_mode_table_on_enter() {
        let p = build_provider(r#"return { submit_mode = { kind = "on_enter" } }"#);
        assert!(matches!(p.submit_mode(), SubmitMode::OnEnter));
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
    fn cancel_defaults_to_close_when_callback_omitted() {
        let mut p = build_provider("return {}");
        assert!(matches!(p.cancel(), PaletteAction::Close));
    }

    #[test]
    fn cancel_invokes_lua_callback() {
        // `cancel` is a side-effect hook (clear a handle, log, …).
        // Verify the Lua callback fires by mutating an upvalue and
        // probing it from outside through `is_loading`, which the
        // provider already round-trips through the same VM.
        let mut p = build_provider(
            r#"
            local cancelled = false
            return {
                cancel = function() cancelled = true end,
                is_loading = function() return cancelled end,
            }
            "#,
        );
        assert!(!p.is_loading());
        let _ = p.cancel();
        assert!(p.is_loading());
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

    #[test]
    fn execute_switch_table_returns_switch_provider_with_new_prompt() {
        // `{ switch = spec }` lets a Lua plugin swap its provider in
        // place — the palette stays open and adopts the new prompt /
        // filter / items pipeline. Verifies the spec is parsed enough
        // to round-trip a basic field (`prompt`).
        let mut p = build_provider(
            r#"
            return {
                execute = function(_)
                    return { switch = { prompt = "theme>" } }
                end,
            }
            "#,
        );
        match p.execute(0, &ctx()) {
            PaletteAction::SwitchProvider(next) => assert_eq!(next.prompt(), "theme>"),
            PaletteAction::Close => panic!("expected SwitchProvider, got Close"),
            PaletteAction::Run(_) => panic!("expected SwitchProvider, got Run"),
            PaletteAction::Push(_) => panic!("expected SwitchProvider, got Push"),
        }
    }

    #[test]
    fn execute_switch_with_non_table_spec_falls_back_to_close() {
        // `switch` value must itself be a table (the new provider's
        // spec). Anything else is malformed — falls through to Close
        // rather than crashing the host.
        let mut p = build_provider(
            r#"
            return {
                execute = function(_) return { switch = "not a table" } end,
            }
            "#,
        );
        assert!(matches!(p.execute(0, &ctx()), PaletteAction::Close));
    }

    #[test]
    fn execute_switch_carries_submit_mode_through_to_new_provider() {
        // The new provider goes through the same `from_spec` path, so
        // its `submit_mode` is read from the switch spec. Confirms the
        // switch path doesn't accidentally drop spec fields.
        let mut p = build_provider(
            r#"
            return {
                execute = function(_)
                    return { switch = { submit_mode = "on_enter" } }
                end,
            }
            "#,
        );
        match p.execute(0, &ctx()) {
            PaletteAction::SwitchProvider(next) => {
                assert!(matches!(next.submit_mode(), SubmitMode::OnEnter));
            }
            _ => panic!("expected SwitchProvider"),
        }
    }
}
