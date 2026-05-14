//! Per-frame `tick` dispatcher.
//!
//! `tick` is the only "event" whose payload (a borrowed `MapApi`)
//! can't be expressed as an [`Event`](crate::event::Event) variant —
//! the Lua-facing `map` table has to be built inside `Lua::scope`
//! against the live ratatui buffer for that frame. That shape needs
//! `mlua` + `MapApi` from `lua/`, so the tick subscribers live in a
//! **separate** registry here (not on the [`EventBus`], which is
//! deliberately free of any `mlua` import).
//!
//! `dispatch` is called once per draw from `app::ui::draw` after
//! composing the map frame.
//!
//! # Identity, removal, reentrancy
//!
//! Each `subscribe` call returns a monotonic `u64`. The Lua surface
//! (`EventHandle`) wraps that ID so a plugin can `:remove()` itself
//! later. Dispatch is **ID snapshot + per-call lookup** — the same
//! pattern the [`EventBus`] uses — so a callback may call
//! [`Self::remove`] (or `subscribe` a new entry) from inside without
//! disturbing the in-flight dispatch.
//!
//! [`EventBus`]: crate::event::EventBus

use std::cell::RefCell;
use std::sync::atomic::{AtomicU64, Ordering};

use mlua::{Lua, RegistryKey};

use crate::lua::MapApi;
use crate::lua::api::map;

/// One subscriber to the per-frame `tick` — a Lua callback paired
/// with the Lua VM it was registered from.
struct TickSubscriber {
    id: u64,
    lua: Lua,
    callback: RegistryKey,
}

/// Lua-only registry of per-frame `tick` callbacks.
///
/// Held by [`crate::lua::LuaHandle`] (which calls [`Self::dispatch`]
/// once per draw) and by the `ttymap.api.frame.on_tick` /
/// `ttymap.on_event("tick", …)` install sites (which call
/// [`Self::subscribe`]). The Lua-facing handle returned to plugins
/// targets [`Self::remove`].
#[derive(Default)]
pub struct TickRegistry {
    subscribers: RefCell<Vec<TickSubscriber>>,
    next_id: AtomicU64,
}

impl TickRegistry {
    /// Register a Lua callback. Returns a monotonic ID
    /// that can be passed to [`Self::remove`].
    pub fn subscribe(&self, lua: Lua, callback: RegistryKey) -> u64 {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        self.subscribers
            .borrow_mut()
            .push(TickSubscriber { id, lua, callback });
        id
    }

    /// Remove one subscriber by ID. Returns true if a matching entry
    /// was found and removed. Safe to call from inside a dispatched
    /// callback — the snapshot-then-lookup pattern in [`Self::dispatch`]
    /// drops its borrow before invoking the Lua function.
    pub fn remove(&self, id: u64) -> bool {
        let mut subs = self.subscribers.borrow_mut();
        let before = subs.len();
        subs.retain(|s| s.id != id);
        before != subs.len()
    }

    /// Total subscriber count.
    pub fn len(&self) -> usize {
        self.subscribers.borrow().len()
    }

    pub fn is_empty(&self) -> bool {
        self.subscribers.borrow().is_empty()
    }

    /// Fire every registered callback against a live `MapApi`.
    ///
    /// Errors are logged + swallowed per callback so a single broken
    /// plugin can't freeze the loop.
    pub fn dispatch(&self, map: &mut MapApi<'_>) {
        let cell = RefCell::new(map);
        let ids: Vec<u64> = self.subscribers.borrow().iter().map(|s| s.id).collect();

        for id in ids {
            // Resolve under a short borrow that drops before the Lua
            // callback runs, so the callback can call `:remove()` (or
            // `:subscribe()` a new entry) on the registry without
            // panicking on a re-borrow.
            let extracted = {
                let subs = self.subscribers.borrow();
                subs.iter().find(|s| s.id == id).and_then(|s| {
                    match s.lua.registry_value::<mlua::Function>(&s.callback) {
                        Ok(f) => Some((s.lua.clone(), f)),
                        Err(e) => {
                            log::warn!("lua: tick subscriber registry lookup failed: {}", e);
                            None
                        }
                    }
                })
            };

            if let Some((lua, f)) = extracted {
                let result: mlua::Result<()> = lua.scope(|scope| {
                    let map_table = map::make_map_table(&lua, scope, &cell)?;
                    f.call::<()>(map_table)
                });
                if let Err(e) = result {
                    log::warn!("lua: tick subscriber failed: {}", e);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::DARK;
    use crate::theme::UiTheme;
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;
    use ttymap_engine::geo::LonLat;
    use ttymap_engine::map::render::frame::MapFrame;
    use ttymap_engine::map::render::overlay::UserPolyline;

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

    fn lua_with_counter(global: &str) -> (mlua::Lua, mlua::RegistryKey) {
        let lua = mlua::Lua::new();
        lua.load(format!(
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
    fn each_subscriber_fires_once_per_dispatch() {
        let (lua_a, key_a) = lua_with_counter("a");
        let (lua_b, key_b) = lua_with_counter("b");

        let ticks = TickRegistry::default();
        ticks.subscribe(lua_a.clone(), key_a);
        ticks.subscribe(lua_b.clone(), key_b);

        let (mut buf, area, frame, theme) = fixture(20, 5);
        let mut sink: Vec<UserPolyline> = Vec::new();
        let mut api = MapApi::new(&mut buf, area, &frame, &theme, None, &mut sink);
        ticks.dispatch(&mut api);

        let a: i64 = lua_a.globals().get("a").expect("read a");
        let b: i64 = lua_b.globals().get("b").expect("read b");
        assert_eq!(a, 1);
        assert_eq!(b, 1);
    }

    #[test]
    fn one_failing_subscriber_does_not_stop_the_others() {
        let lua_bad = mlua::Lua::new();
        lua_bad
            .load(r#"function bang(_map) error("boom") end"#)
            .exec()
            .unwrap();
        let bad_fn: mlua::Function = lua_bad.globals().get("bang").unwrap();
        let bad_key = lua_bad.create_registry_value(bad_fn).unwrap();

        let (lua_good, good_key) = lua_with_counter("good");

        let ticks = TickRegistry::default();
        ticks.subscribe(lua_bad, bad_key);
        ticks.subscribe(lua_good.clone(), good_key);

        let (mut buf, area, frame, theme) = fixture(20, 5);
        let mut sink: Vec<UserPolyline> = Vec::new();
        let mut api = MapApi::new(&mut buf, area, &frame, &theme, None, &mut sink);
        ticks.dispatch(&mut api);

        let good: i64 = lua_good.globals().get("good").unwrap();
        assert_eq!(good, 1);
    }

    #[test]
    fn remove_drops_the_subscriber() {
        let (lua, key) = lua_with_counter("c");
        let ticks = TickRegistry::default();
        let id = ticks.subscribe(lua.clone(), key);

        let (mut buf, area, frame, theme) = fixture(20, 5);
        let mut sink: Vec<UserPolyline> = Vec::new();
        let mut api = MapApi::new(&mut buf, area, &frame, &theme, None, &mut sink);
        ticks.dispatch(&mut api);

        assert!(ticks.remove(id));
        ticks.dispatch(&mut api);

        let c: i64 = lua.globals().get("c").expect("read c");
        assert_eq!(c, 1, "removed subscriber must not fire on next dispatch");
        assert!(!ticks.remove(id), "second remove returns false");
    }
}
