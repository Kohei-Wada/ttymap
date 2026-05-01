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

use mlua::Table;

use super::handle::{CallOutcome, LuaHandle, fresh_load};
use crate::app::AppMsg;
use crate::compositor::Context;
use crate::geo::LonLat;
use crate::lua::ttymap::{CapturedKind, LuaHostShared};
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
    pub fn from_source(
        source: &'static str,
        id: &'static str,
        shared: Arc<LuaHostShared>,
    ) -> mlua::Result<Box<Self>> {
        // Per-instance Lua state — `ttymap.plugin` is not exposed
        // here. The provider closes itself by returning a
        // `PaletteAction::Close` from `execute`.
        let (lua, captured, handles) = fresh_load(source, id, "lua-palette", shared, None)?;
        // The script self-declares as a palette provider via
        // `ttymap.register_palette(...)`. A `register_plugin` call
        // here is a kind mismatch reported up to the walker.
        let palette = match captured.kind {
            Some(CapturedKind::Palette(t)) => t,
            Some(CapturedKind::Plugin(_)) | Some(CapturedKind::Overlay(_)) => {
                return Err(mlua::Error::external(
                    "expected ttymap.register_palette, got ttymap.register_plugin/register_overlay",
                ));
            }
            None => {
                return Err(mlua::Error::external(
                    "script did not call any ttymap.register_* API",
                ));
            }
        };

        let prompt: String = palette.get("prompt").unwrap_or_else(|_| ":".to_string());
        let submit_mode = parse_submit_mode(&palette);
        let handle = LuaHandle::new(lua, palette, id)?;

        Ok(Box::new(Self {
            handle,
            prompt,
            submit_mode,
            items: Vec::new(),
            jump_rx: handles.jump_rx,
        }))
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
    use crate::theme::ThemeId;

    fn ctx() -> Context {
        Context {
            center: LonLat { lon: 0.0, lat: 0.0 },
            theme_id: ThemeId::Dark,
            cursor: None,
        }
    }

    #[test]
    fn prompt_falls_back_to_colon_when_palette_omits_it() {
        let p = LuaPaletteProvider::from_source(
            "ttymap.register_palette({})",
            "anon",
            LuaHostShared::empty(),
        )
        .expect("load");
        assert_eq!(p.prompt(), ":");
    }

    #[test]
    fn prompt_picks_up_palette_value() {
        let p = LuaPaletteProvider::from_source(
            r#"ttymap.register_palette({ prompt = "/" })"#,
            "named",
            LuaHostShared::empty(),
        )
        .expect("load");
        assert_eq!(p.prompt(), "/");
    }

    #[test]
    fn from_source_rejects_when_script_calls_register_plugin_instead() {
        // The script self-declares as a Component, but the palette
        // adapter expects a palette declaration — kind mismatch
        // surfaces as an error rather than a silent miscoercion.
        let err = LuaPaletteProvider::from_source(
            "ttymap.register_plugin({})",
            "wrong-kind",
            LuaHostShared::empty(),
        );
        assert!(
            err.is_err(),
            "register_plugin should fail to load as a palette provider"
        );
    }

    #[test]
    fn submit_mode_defaults_on_each_key() {
        let p = LuaPaletteProvider::from_source(
            "ttymap.register_palette({})",
            "anon",
            LuaHostShared::empty(),
        )
        .expect("load");
        assert!(matches!(p.submit_mode(), SubmitMode::OnEachKey));
    }

    #[test]
    fn submit_mode_string_debounced_uses_default_ms() {
        let p = LuaPaletteProvider::from_source(
            r#"ttymap.register_palette({ submit_mode = "debounced" })"#,
            "anon",
            LuaHostShared::empty(),
        )
        .expect("load");
        match p.submit_mode() {
            SubmitMode::Debounced(d) => assert_eq!(d, Duration::from_millis(400)),
            _ => panic!("expected Debounced"),
        }
    }

    #[test]
    fn submit_mode_table_lets_plugin_pick_ms() {
        let p = LuaPaletteProvider::from_source(
            r#"ttymap.register_palette({ submit_mode = { kind = "debounced", ms = 250 } })"#,
            "anon",
            LuaHostShared::empty(),
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
            ttymap.register_palette({
                filter = function(q)
                    items_list = {}
                    if q ~= "" then
                        table.insert(items_list, { label = q .. " a", hint = "ha" })
                        table.insert(items_list, { label = q .. " b", hint = "hb" })
                    end
                end,
                items = function() return items_list end,
            })
            "#,
            "round-trip",
            LuaHostShared::empty(),
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
            ttymap.register_palette({
                execute = function(idx)
                    ttymap.map:jump(139.7, 35.7)
                    return nil
                end,
            })
            "#,
            "exec-jump",
            LuaHostShared::empty(),
        )
        .expect("load");
        match p.execute(0, &ctx()) {
            PaletteAction::Run(msgs) => {
                assert_eq!(msgs.len(), 1);
                assert!(matches!(msgs[0], AppMsg::Map(crate::map::Action::Jump(_))));
            }
            _ => panic!("expected Run([Jump])"),
        }
    }

    #[test]
    fn execute_close_table_returns_close() {
        let mut p = LuaPaletteProvider::from_source(
            r#"ttymap.register_palette({ execute = function(_) return { close = true } end })"#,
            "exec-close",
            LuaHostShared::empty(),
        )
        .expect("load");
        assert!(matches!(p.execute(0, &ctx()), PaletteAction::Close));
    }

    #[test]
    fn is_loading_defaults_false() {
        let p = LuaPaletteProvider::from_source(
            "ttymap.register_palette({})",
            "anon",
            LuaHostShared::empty(),
        )
        .expect("load");
        assert!(!p.is_loading());
    }

    #[test]
    fn is_loading_reads_palette_function() {
        let p = LuaPaletteProvider::from_source(
            r#"ttymap.register_palette({ is_loading = function() return true end })"#,
            "loading",
            LuaHostShared::empty(),
        )
        .expect("load");
        assert!(p.is_loading());
    }
}
