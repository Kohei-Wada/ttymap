//! Per-frame tick dispatcher.
//!
//! Each `ttymap.api.frame.on_tick(fn)` call from a plugin script
//! lands here as a [`TickEntry`]. The App's main thread calls
//! [`LuaTickRegistry::tick`] once per frame, which walks the registry
//! and dispatches each callback against a per-frame `MapApi` table.
//!
//! This is the unified per-frame work mechanism for the nvim-style
//! plugin API: plugins that paint markers, drain async fetches, or
//! do periodic work all use the same `on_tick` subscription. Errors
//! from one callback are logged and swallowed so a single broken
//! plugin cannot freeze the host.

use mlua::{Lua, RegistryKey};

use crate::compositor::MapApi;
use crate::lua::ttymap::map_api;

/// One registered per-frame callback. The Lua state is the plugin's
/// setup state (cloned from `register_one`); the registry key points
/// at the function passed to `ttymap.api.frame.on_tick`.
pub struct TickEntry {
    pub name: &'static str,
    pub lua: Lua,
    pub callback: RegistryKey,
}

/// Registry of every plugin-declared per-frame `on_tick` callback.
/// Built up by `register_one` as bundled and user plugins load,
/// then ticked once per frame from `App::run`.
#[derive(Default)]
pub struct LuaTickRegistry {
    entries: Vec<TickEntry>,
}

impl LuaTickRegistry {
    pub fn add(&mut self, entry: TickEntry) {
        self.entries.push(entry);
    }

    /// Number of registered tick callbacks. Used by tests and could
    /// be useful to surface in diagnostics.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Run every registered callback once. Errors are logged and the
    /// loop continues — a single broken plugin must not freeze the app.
    ///
    /// `MapApi` borrows the ratatui buffer for one frame, so the
    /// Lua-facing handle is built inside `Lua::scope` (closures over
    /// a `RefCell` of the ref) and torn down before this method
    /// returns.
    pub fn tick(&self, map: &mut MapApi<'_>) {
        let cell = std::cell::RefCell::new(map);
        for entry in &self.entries {
            let result: mlua::Result<()> = entry.lua.scope(|scope| {
                let map_table = map_api::make_map_table(&entry.lua, scope, &cell)?;
                let f: mlua::Function = entry.lua.registry_value(&entry.callback)?;
                f.call::<()>(map_table)
            });
            if let Err(e) = result {
                log::warn!("lua[{}]: on_tick failed: {}", entry.name, e);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geo::LonLat;
    use crate::map::render::frame::MapFrame;
    use crate::map::render::overlay::UserPolyline;
    use crate::theme::{DARK, UiTheme};
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;

    fn fixture(area_w: u16, area_h: u16) -> (Buffer, Rect, MapFrame, UiTheme) {
        let area = Rect::new(0, 0, area_w, area_h);
        let buf = Buffer::empty(area);
        let frame = MapFrame {
            cells: Vec::new(),
            cols: area_w,
            rows: area_h,
            center: LonLat { lon: 0.0, lat: 0.0 },
            zoom: 1.0,
        };
        let theme = UiTheme::from_palette(&DARK);
        (buf, area, frame, theme)
    }

    /// Build a fresh Lua state with a counter closure stashed in the
    /// registry. Returns the state, the registry key for the
    /// callback, and the global name where the counter lives so the
    /// test can read it back.
    fn lua_with_counter(global: &str) -> (Lua, RegistryKey) {
        let lua = Lua::new();
        // Counter starts at 0; each call bumps it by 1.
        lua.load(&format!(
            r#"
            {global} = 0
            function tick_{global}(_map)
                {global} = {global} + 1
            end
            "#
        ))
        .exec()
        .expect("lua exec");
        let f: mlua::Function = lua.globals().get(format!("tick_{global}")).expect("get fn");
        let key = lua.create_registry_value(f).expect("registry");
        (lua, key)
    }

    #[test]
    fn tick_calls_each_registered_callback_once_per_call() {
        let (lua_a, key_a) = lua_with_counter("a");
        let (lua_b, key_b) = lua_with_counter("b");

        let mut reg = LuaTickRegistry::default();
        reg.add(TickEntry {
            name: "a",
            lua: lua_a.clone(),
            callback: key_a,
        });
        reg.add(TickEntry {
            name: "b",
            lua: lua_b.clone(),
            callback: key_b,
        });
        assert_eq!(reg.len(), 2);

        let (mut buf, area, frame, theme) = fixture(20, 5);
        {
            let mut sink: Vec<UserPolyline> = Vec::new();
            let mut api = MapApi::new(&mut buf, area, &frame, &theme, None, &mut sink);
            reg.tick(&mut api);
        }
        let a: i64 = lua_a.globals().get("a").expect("read a");
        let b: i64 = lua_b.globals().get("b").expect("read b");
        assert_eq!(a, 1, "first tick should bump a once");
        assert_eq!(b, 1, "first tick should bump b once");

        {
            let mut sink: Vec<UserPolyline> = Vec::new();
            let mut api = MapApi::new(&mut buf, area, &frame, &theme, None, &mut sink);
            reg.tick(&mut api);
        }
        let a: i64 = lua_a.globals().get("a").expect("read a");
        let b: i64 = lua_b.globals().get("b").expect("read b");
        assert_eq!(a, 2, "second tick should bump a again");
        assert_eq!(b, 2, "second tick should bump b again");
    }

    /// A callback that throws is logged and swallowed; subsequent
    /// callbacks in the same registry still fire. Guards against the
    /// "one buggy plugin freezes everyone" failure mode.
    #[test]
    fn tick_continues_after_a_callback_errors() {
        let lua_bad = Lua::new();
        lua_bad
            .load(r#"function bang(_map) error("boom") end"#)
            .exec()
            .expect("lua exec");
        let bad_fn: mlua::Function = lua_bad.globals().get("bang").expect("get bang");
        let bad_key = lua_bad.create_registry_value(bad_fn).expect("registry");

        let (lua_good, good_key) = lua_with_counter("good");

        let mut reg = LuaTickRegistry::default();
        reg.add(TickEntry {
            name: "bad",
            lua: lua_bad,
            callback: bad_key,
        });
        reg.add(TickEntry {
            name: "good",
            lua: lua_good.clone(),
            callback: good_key,
        });

        let (mut buf, area, frame, theme) = fixture(20, 5);
        let mut sink: Vec<UserPolyline> = Vec::new();
        let mut api = MapApi::new(&mut buf, area, &frame, &theme, None, &mut sink);
        reg.tick(&mut api);

        let good: i64 = lua_good.globals().get("good").expect("read good");
        assert_eq!(
            good, 1,
            "broken upstream callback should not stop downstream callbacks"
        );
    }

    #[test]
    fn empty_registry_tick_is_a_noop() {
        let reg = LuaTickRegistry::default();
        let (mut buf, area, frame, theme) = fixture(20, 5);
        let mut sink: Vec<UserPolyline> = Vec::new();
        let mut api = MapApi::new(&mut buf, area, &frame, &theme, None, &mut sink);
        reg.tick(&mut api);
        assert!(reg.is_empty());
    }
}
