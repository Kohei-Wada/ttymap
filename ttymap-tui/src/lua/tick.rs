//! Per-frame `tick` dispatcher.
//!
//! `tick` is the only event whose payload (a borrowed `MapApi`) can't
//! be expressed as an [`Event`](crate::event::Event) variant — the
//! Lua-facing `map` table has to be built inside `Lua::scope` against
//! the live ratatui buffer for that frame. That shape needs `mlua` +
//! `MapApi` from `lua/`, so it lives here rather than in `event/`
//! (which deliberately stays free of `crate::lua::*` imports).
//!
//! Called once per draw from `app::ui::draw` after composing the map
//! frame.

use std::cell::RefCell;

use crate::event::EventBus;
use crate::lua::MapApi;
use crate::lua::api::map;

/// Fire the per-frame `"tick"` bucket against a live `MapApi`. Each
/// Lua subscriber sees the per-frame map table; Rust subscribers in
/// the bucket are skipped (a Rust thing wanting per-frame work
/// should subscribe to `frame_ready` instead).
///
/// Errors are logged + swallowed per callback so a single broken
/// plugin can't freeze the loop.
pub fn dispatch_tick(bus: &EventBus, map: &mut MapApi<'_>) {
    let cell = RefCell::new(map);
    bus.for_each_lua_subscriber("tick", |lua, f| {
        let result: mlua::Result<()> = lua.scope(|scope| {
            let map_table = map::make_map_table(lua, scope, &cell)?;
            f.call::<()>(map_table)
        });
        if let Err(e) = result {
            log::warn!("lua: tick subscriber failed: {}", e);
        }
    });
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

        let bus = EventBus::default();
        bus.subscribe_lua("tick", lua_a.clone(), key_a);
        bus.subscribe_lua("tick", lua_b.clone(), key_b);

        let (mut buf, area, frame, theme) = fixture(20, 5);
        let mut sink: Vec<UserPolyline> = Vec::new();
        let mut api = MapApi::new(&mut buf, area, &frame, &theme, None, &mut sink);
        dispatch_tick(&bus, &mut api);

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

        let bus = EventBus::default();
        bus.subscribe_lua("tick", lua_bad, bad_key);
        bus.subscribe_lua("tick", lua_good.clone(), good_key);

        let (mut buf, area, frame, theme) = fixture(20, 5);
        let mut sink: Vec<UserPolyline> = Vec::new();
        let mut api = MapApi::new(&mut buf, area, &frame, &theme, None, &mut sink);
        dispatch_tick(&bus, &mut api);

        let good: i64 = lua_good.globals().get("good").unwrap();
        assert_eq!(good, 1);
    }
}
