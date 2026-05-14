//! `ttymap.map` — Lua bridges for the map surface.
//!
//! Two distinct objects, both surfaced as `map` in Lua:
//!
//! - **[`HostMap`]** — persistent userdata installed as `ttymap.map`
//!   in the global. Mutators (`jump`, `zoom(level)`, `fly_to`) are
//!   fire-and-forget; each enqueues an
//!   [`Op::Command(UserCommand::Map(...))`] onto the shared
//!   [`OpsBuffer`](crate::compositor::op::OpsBuffer). Read methods
//!   (`center`, no-arg `zoom`) consult shared `Arc<Mutex<...>>` cells
//!   the host refreshes on every dispatch path that carries a
//!   `Window` / `MapApi`.
//! - **[`make_map_table`]** — per-frame Lua table built inside
//!   `Lua::scope` and handed to `on_tick` callbacks as the `map`
//!   parameter. Drawing primitives (`point`, `label`, `text_anchored`,
//!   `polyline`) plus read-only frame state (`center`, `zoom`,
//!   `area_width`, `cursor`) plus theme accessors. `MapApi` carries a
//!   non-`'static` lifetime so we can't use
//!   `Scope::create_userdata_ref_mut`; instead the per-frame table
//!   wraps `scope.create_function` closures over a `RefCell` of the
//!   `MapApi` ref.
//!
//! Color args: `point` / `label` / `polyline` / `text_anchored`
//! accept either a theme-aware keyword (`"accent"` | `"accent_alt"` |
//! `"muted"` | `"road"`) or a direct xterm-256 integer index
//! (0..=255). Palette accessor methods (`accent_color`, `road_color`,
//! etc.) return xterm indices for round-tripping.

use std::cell::RefCell;
use std::sync::{Arc, Mutex};

use mlua::{Lua, Scope, Table, UserData};

use crate::UserCommand;
use crate::compositor::op::{Op, OpsBuffer};
use crate::lua::MapApi;
use crate::lua::map_api::Anchor;
use ttymap_engine::geo::LonLat;
use ttymap_engine::map::MapAction;

// ── ttymap.map (persistent userdata) ────────────────────────────────

pub(super) struct HostMap {
    /// Shared op buffer the lua subsystem drains every iteration.
    /// Fire-and-forget Lua intents (`jump` / `zoom` / `fly_to`)
    /// enqueue an `Op::Command(UserCommand::Map(...))`; the host treats
    /// them identically to a keymap-driven dispatch.
    ops: OpsBuffer,
    center: Arc<Mutex<LonLat>>,
    zoom: Arc<Mutex<f64>>,
}

impl HostMap {
    pub(super) fn new(ops: OpsBuffer, center: Arc<Mutex<LonLat>>, zoom: Arc<Mutex<f64>>) -> Self {
        Self { ops, center, zoom }
    }
}

impl UserData for HostMap {
    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        // `ttymap.map:jump(lon, lat)` — request the map recentre on
        // the given coordinate. Enqueues `UserCommand::Map(Jump)` onto
        // the shared op buffer so the host treats it identically to a
        // keymap-driven jump.
        methods.add_method("jump", |_, this, (lon, lat): (f64, f64)| {
            this.ops
                .borrow_mut()
                .push(Op::Command(UserCommand::Map(MapAction::Jump(LonLat {
                    lon,
                    lat,
                }))));
            Ok(())
        });

        // `ttymap.map:zoom([level])` — overloaded:
        //   `:zoom(level)` queues a zoom request (clamped host-side
        //   in `MapState::process_action`). Fire-and-forget.
        //   `:zoom()` (no arg) returns the current zoom level read
        //   from the shared `Arc<Mutex<f64>>` the host refreshes on
        //   the same dispatch paths it refreshes `:center()` on.
        // mlua dispatches by the supplied argument signature: nil →
        // `None` (getter), number → `Some(level)` (setter).
        methods.add_method("zoom", |_, this, level: Option<f64>| match level {
            Some(z) => {
                this.ops
                    .borrow_mut()
                    .push(Op::Command(UserCommand::Map(MapAction::SetZoom(z))));
                Ok(mlua::Value::Nil)
            }
            None => {
                let z = *this.zoom.lock().expect("zoom mutex poisoned");
                Ok(mlua::Value::Number(z))
            }
        });

        // `ttymap.map:fly_to(lon, lat, zoom)` — composite recenter +
        // zoom in one dispatch. Emitting `jump` + `zoom` separately
        // would render two frames; this routes through `MapFlyTo`
        // so the user sees a single transition.
        methods.add_method("fly_to", |_, this, (lon, lat, zoom): (f64, f64, f64)| {
            this.ops
                .borrow_mut()
                .push(Op::Command(UserCommand::Map(MapAction::FlyTo {
                    center: LonLat { lon, lat },
                    zoom,
                })));
            Ok(())
        });

        // `ttymap.map:center()` -> lon, lat — current map centre, kept
        // fresh by the host before each dispatch path that carries a
        // `Window` / `MapApi`. Plugins use this to scope upstream
        // queries (e.g. an OpenSky bounding box around the user's
        // view).
        methods.add_method("center", |_, this, _: ()| {
            let ll = *this.center.lock().expect("center mutex poisoned");
            Ok((ll.lon, ll.lat))
        });

        // `ttymap.map:set_labels_visible(b)` — show / hide every
        // tile-rendered text label (place names, road names …).
        // Geometry features (roads, water, fills) keep rendering.
        // Used by `geo_quiz` hard mode to suppress city-name hints;
        // any plugin can flip it for screenshot-style clean views.
        // The flag flips on the render thread and a redraw fires
        // automatically (the dispatcher pairs it with
        // `request_map_redraw`).
        methods.add_method("set_labels_visible", |_, this, visible: bool| {
            this.ops
                .borrow_mut()
                .push(Op::Command(UserCommand::SetLabelsVisible(visible)));
            Ok(())
        });
    }
}

// ── per-frame draw table (on_tick callback `map` parameter) ─────────

/// Build the Lua-facing `map` table for a single per-frame `on_tick`
/// call. The closures borrow `cell` for `'scope`; once the host's
/// `Lua::scope` returns the closures are dropped and the borrow is
/// released, so it's safe to take the ratatui buffer back out.
pub(crate) fn make_map_table<'scope, 'lua_scope>(
    lua: &Lua,
    scope: &'scope Scope<'scope, 'lua_scope>,
    cell: &'scope RefCell<&mut MapApi<'_>>,
) -> mlua::Result<Table>
where
    'lua_scope: 'scope,
{
    let table = lua.create_table()?;
    table.set("point", scope.create_function(|_, args| point(cell, args))?)?;
    table.set("label", scope.create_function(|_, args| label(cell, args))?)?;
    table.set(
        "text_anchored",
        scope.create_function(|_, args| text_anchored(cell, args))?,
    )?;
    table.set(
        "polyline",
        scope.create_function(|_, args| polyline(cell, args))?,
    )?;
    table.set(
        "center",
        scope.create_function(|_, _: mlua::Table| {
            let p = cell.borrow();
            let ll = p.center();
            Ok((ll.lon, ll.lat))
        })?,
    )?;
    table.set(
        "zoom",
        scope.create_function(|_, _: mlua::Table| Ok(cell.borrow().zoom()))?,
    )?;
    table.set(
        "area_width",
        scope.create_function(|_, _: mlua::Table| Ok(cell.borrow().area_width()))?,
    )?;
    // `map:cursor() -> lon, lat | nil, nil` — returns two values so
    // a plugin can `local lon, lat = map:cursor() if lon then ... end`
    // without unwrapping a tuple. `Option<(f64,f64)>` doesn't satisfy
    // mlua's `IntoLuaMulti`, hence the pair of `Option<f64>`.
    table.set(
        "cursor",
        scope.create_function(|_, _: mlua::Table| match cell.borrow().cursor() {
            Some(ll) => Ok((Some(ll.lon), Some(ll.lat))),
            None => Ok((None, None)),
        })?,
    )?;
    // Palette colour accessors — return the active theme's colour as an
    // xterm-256 index so plugins can pass them back into `map:polyline`
    // or compare colours at runtime.
    table.set(
        "accent_color",
        scope.create_function(|_, _: mlua::Table| Ok(cell.borrow().accent_color_xterm()))?,
    )?;
    table.set(
        "accent_alt_color",
        scope.create_function(|_, _: mlua::Table| Ok(cell.borrow().accent_alt_color_xterm()))?,
    )?;
    table.set(
        "muted_color",
        scope.create_function(|_, _: mlua::Table| Ok(cell.borrow().muted_color_xterm()))?,
    )?;
    table.set(
        "road_color",
        scope.create_function(|_, _: mlua::Table| Ok(cell.borrow().road_color_xterm()))?,
    )?;
    Ok(table)
}

/// Parse a Lua-side anchor keyword. Hyphenated form matches the
/// `layout.anchor` convention used elsewhere in the bridge.
/// Unknown values default to `TopLeft` so a typo paints somewhere
/// visible instead of being silently dropped.
fn anchor_from_str(s: &str) -> Anchor {
    match s {
        "top-right" | "topright" => Anchor::TopRight,
        "bottom-left" | "bottomleft" => Anchor::BottomLeft,
        "bottom-right" | "bottomright" => Anchor::BottomRight,
        _ => Anchor::TopLeft,
    }
}

/// Resolve a Lua-side colour argument used by all three drawing
/// primitives (`point`, `label`, `polyline`).
///
/// Accepts:
/// - **`nil`** → default to the theme's accent colour.
/// - **String keyword** → `"accent"` / `"accent_alt"` / `"muted"` /
///   `"road"`, resolved through the active palette. Unknown keywords
///   fall back to accent.
/// - **Integer** → used as a raw xterm-256 palette index (0..=255).
///   Out-of-range values are clamped (negative → 0, >255 → 255).
///
/// Anything else (table, function, …) falls back to accent.
fn resolve_color_arg(p: &MapApi<'_>, arg: Option<&mlua::Value>) -> u8 {
    match arg {
        Some(mlua::Value::Integer(n)) => (*n).clamp(0, 255) as u8,
        Some(mlua::Value::String(s)) => resolve_keyword(p, s),
        _ => xterm_index(p.accent_color()),
    }
}

fn resolve_keyword(p: &MapApi<'_>, keyword: &mlua::String) -> u8 {
    match keyword.to_str().as_deref() {
        Ok("road") => p.road_color_xterm(),
        Ok("accent_alt") => xterm_index(p.accent_alt_color()),
        Ok("muted") => xterm_index(p.muted_color()),
        _ => xterm_index(p.accent_color()),
    }
}

fn xterm_index(color: ratatui::style::Color) -> u8 {
    match color {
        ratatui::style::Color::Indexed(i) => i,
        _ => 7,
    }
}

/// Thin wrapper around [`resolve_color_arg`] that returns a
/// `ratatui::style::Color::Indexed` for use by buffer-side primitives
/// (`point`, `label`) that take a `Color` directly.
fn resolve_color_value(p: &MapApi<'_>, arg: Option<&mlua::Value>) -> ratatui::style::Color {
    ratatui::style::Color::Indexed(resolve_color_arg(p, arg))
}

// First param is the receiver from Lua's `map:point(...)` colon
// syntax — `map:point(a, b, c)` desugars to `map.point(map, a, b, c)`,
// so the closure sees the map table itself as its first argument.
// We discard it; the actual `MapApi` handle is the captured `cell`.
fn point(
    cell: &RefCell<&mut MapApi<'_>>,
    (_self, lon, lat, glyph, color_arg): (mlua::Table, f64, f64, mlua::String, Option<mlua::Value>),
) -> mlua::Result<()> {
    let glyph_str = glyph.to_str()?;
    let glyph_char = glyph_str.chars().next().unwrap_or(' ');
    let mut p = cell.borrow_mut();
    let color = resolve_color_value(&p, color_arg.as_ref());
    p.point(LonLat { lon, lat }, glyph_char, color);
    Ok(())
}

// `map:text_anchored(anchor, rows_in, text, color?)` — paints `text`
// at one of the four screen-space corners. Anchor is a hyphenated
// keyword ("top-left" / "top-right" / "bottom-left" / "bottom-right");
// `rows_in` offsets toward the interior.
fn text_anchored(
    cell: &RefCell<&mut MapApi<'_>>,
    (_self, anchor, rows_in, text, color_arg): (
        mlua::Table,
        mlua::String,
        u16,
        mlua::String,
        Option<mlua::Value>,
    ),
) -> mlua::Result<()> {
    let anchor_str = anchor.to_str()?;
    let anchor = anchor_from_str(&anchor_str);
    let text_str = text.to_str()?;
    let mut p = cell.borrow_mut();
    let color = resolve_color_value(&p, color_arg.as_ref());
    p.text_anchored(anchor, rows_in, &text_str, color);
    Ok(())
}

// `map:label(lon, lat, text, color?)` — paints `text` starting one
// cell to the right of the projected point. Multi-cell text picks up
// the same colour fallback as `point`.
fn label(
    cell: &RefCell<&mut MapApi<'_>>,
    (_self, lon, lat, text, color_arg): (mlua::Table, f64, f64, mlua::String, Option<mlua::Value>),
) -> mlua::Result<()> {
    let text_str = text.to_str()?;
    let mut p = cell.borrow_mut();
    let color = resolve_color_value(&p, color_arg.as_ref());
    p.label(LonLat { lon, lat }, &text_str, color);
    Ok(())
}

// `map:polyline(coords, color?)` — coords is a Lua sequence of
// `{lon, lat}` pairs; color is either a theme keyword string
// (`"accent"` / `"accent_alt"` / `"muted"` / `"road"`) or a direct
// xterm-256 integer index (0..=255). Length-1 coords are silently
// dropped (matches `point`/`label`'s off-canvas behaviour). The
// polyline is queued for the next frame's render task — there is a
// 1-frame latency.
fn polyline(
    cell: &RefCell<&mut MapApi<'_>>,
    (_self, coords_table, color_arg): (mlua::Table, mlua::Table, Option<mlua::Value>),
) -> mlua::Result<()> {
    let mut coords: Vec<LonLat> = Vec::new();
    for pair_res in coords_table.sequence_values::<mlua::Table>() {
        let pair = pair_res?;
        let lon: f64 = pair.get(1)?;
        let lat: f64 = pair.get(2)?;
        coords.push(LonLat { lon, lat });
    }
    if coords.len() < 2 {
        return Ok(());
    }
    let mut p = cell.borrow_mut();
    let color = resolve_color_arg(&p, color_arg.as_ref());
    p.push_polyline_overlay(coords, color);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lua::MapApi;
    use crate::theme::DARK;
    use crate::theme::UiTheme;
    use mlua::Lua;
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;
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

    /// `map:polyline({{0,0},{1,1}}, "accent")` pushes one entry into the
    /// overlay sink with both points and the resolved accent colour.
    #[test]
    fn polyline_pushes_to_sink() {
        let (mut buf, area, frame, theme) = fixture(40, 10);
        let mut sink: Vec<UserPolyline> = Vec::new();
        let mut api = MapApi::new(&mut buf, area, &frame, &theme, None, &mut sink);
        let lua = Lua::new();
        let cell = std::cell::RefCell::new(&mut api);
        lua.scope(|scope| {
            let map_table = make_map_table(&lua, scope, &cell)?;
            lua.globals().set("map", map_table)?;
            lua.load(r#"map:polyline({{0,0},{1,1}}, "accent")"#).exec()
        })
        .expect("scope");
        assert_eq!(sink.len(), 1);
        assert_eq!(sink[0].coords.len(), 2);
        assert_eq!(sink[0].coords[0], LonLat { lon: 0.0, lat: 0.0 });
        assert_eq!(sink[0].coords[1], LonLat { lon: 1.0, lat: 1.0 });
    }

    /// Single-point polyline is a silent drop — sink stays empty.
    #[test]
    fn polyline_with_single_point_is_dropped() {
        let (mut buf, area, frame, theme) = fixture(40, 10);
        let mut sink: Vec<UserPolyline> = Vec::new();
        let mut api = MapApi::new(&mut buf, area, &frame, &theme, None, &mut sink);
        let lua = Lua::new();
        let cell = std::cell::RefCell::new(&mut api);
        lua.scope(|scope| {
            let map_table = make_map_table(&lua, scope, &cell)?;
            lua.globals().set("map", map_table)?;
            lua.load(r#"map:polyline({{0,0}})"#).exec()
        })
        .expect("scope");
        assert!(sink.is_empty());
    }

    /// `"road"` keyword resolves to `palette.road_motorway` — the same
    /// colour the map renderer uses for motorway-class roads.
    #[test]
    fn polyline_road_keyword_resolves_to_road_motorway() {
        let (mut buf, area, frame, theme) = fixture(40, 10);
        let mut sink: Vec<UserPolyline> = Vec::new();
        let road_idx = DARK.road_motorway;
        let mut api = MapApi::new(&mut buf, area, &frame, &theme, None, &mut sink);
        let lua = Lua::new();
        let cell = std::cell::RefCell::new(&mut api);
        lua.scope(|scope| {
            let map_table = make_map_table(&lua, scope, &cell)?;
            lua.globals().set("map", map_table)?;
            lua.load(r#"map:polyline({{0,0},{1,1}}, "road")"#).exec()
        })
        .expect("scope");
        assert_eq!(sink.len(), 1);
        assert_eq!(
            sink[0].color, road_idx,
            "the \"road\" keyword must resolve to palette.road_motorway"
        );
    }

    /// Unknown colour keyword falls back to "accent" — same behaviour
    /// as `resolve_color` for `point`/`label`.
    #[test]
    fn polyline_unknown_color_falls_back_to_accent() {
        let (mut buf, area, frame, theme) = fixture(40, 10);
        let mut sink: Vec<UserPolyline> = Vec::new();
        let accent_idx = DARK.accent;
        let mut api = MapApi::new(&mut buf, area, &frame, &theme, None, &mut sink);
        let lua = Lua::new();
        let cell = std::cell::RefCell::new(&mut api);
        lua.scope(|scope| {
            let map_table = make_map_table(&lua, scope, &cell)?;
            lua.globals().set("map", map_table)?;
            lua.load(r#"map:polyline({{0,0},{1,1}}, "zzzz")"#).exec()
        })
        .expect("scope");
        assert_eq!(sink.len(), 1);
        assert_eq!(sink[0].color, accent_idx);
    }

    /// `map:polyline({coords}, 222)` accepts an integer as a direct
    /// xterm-256 index.
    #[test]
    fn polyline_accepts_integer_xterm_index() {
        let (mut buf, area, frame, theme) = fixture(40, 10);
        let mut sink: Vec<UserPolyline> = Vec::new();
        let mut api = MapApi::new(&mut buf, area, &frame, &theme, None, &mut sink);
        let lua = Lua::new();
        let cell = std::cell::RefCell::new(&mut api);
        lua.scope(|scope| {
            let map_table = make_map_table(&lua, scope, &cell)?;
            lua.globals().set("map", map_table)?;
            lua.load(r#"map:polyline({{0,0},{1,1}}, 222)"#).exec()
        })
        .expect("scope");
        assert_eq!(sink.len(), 1);
        assert_eq!(
            sink[0].color, 222,
            "integer arg used as direct xterm-256 index"
        );
    }

    /// Out-of-range integer arguments clamp into 0..=255.
    #[test]
    fn polyline_clamps_out_of_range_integer_color() {
        let (mut buf, area, frame, theme) = fixture(40, 10);
        let mut sink: Vec<UserPolyline> = Vec::new();
        let mut api = MapApi::new(&mut buf, area, &frame, &theme, None, &mut sink);
        let lua = Lua::new();
        let cell = std::cell::RefCell::new(&mut api);
        lua.scope(|scope| {
            let map_table = make_map_table(&lua, scope, &cell)?;
            lua.globals().set("map", map_table)?;
            lua.load(r#"map:polyline({{0,0},{1,1}}, -50); map:polyline({{0,0},{1,1}}, 500)"#)
                .exec()
        })
        .expect("scope");
        assert_eq!(sink.len(), 2);
        assert_eq!(sink[0].color, 0, "negative integer clamps to 0");
        assert_eq!(sink[1].color, 255, ">255 integer clamps to 255");
    }

    /// `map:road_color()` returns the active palette's road_motorway
    /// xterm-256 index. Verified against `DARK.road_motorway` (= 222).
    #[test]
    fn road_color_accessor_returns_palette_road_motorway() {
        let (mut buf, area, frame, theme) = fixture(40, 10);
        let mut sink: Vec<UserPolyline> = Vec::new();
        let mut api = MapApi::new(&mut buf, area, &frame, &theme, None, &mut sink);
        let lua = Lua::new();
        let cell = std::cell::RefCell::new(&mut api);
        let result: u8 = lua
            .scope(|scope| {
                let map_table = make_map_table(&lua, scope, &cell)?;
                lua.globals().set("map", map_table)?;
                lua.load(r#"return map:road_color()"#).eval::<u8>()
            })
            .expect("scope");
        assert_eq!(result, DARK.road_motorway, "matches DARK.road_motorway");
    }

    /// Plugins can chain accessor → polyline call.
    #[test]
    fn polyline_accepts_road_color_accessor_result() {
        let (mut buf, area, frame, theme) = fixture(40, 10);
        let mut sink: Vec<UserPolyline> = Vec::new();
        let mut api = MapApi::new(&mut buf, area, &frame, &theme, None, &mut sink);
        let lua = Lua::new();
        let cell = std::cell::RefCell::new(&mut api);
        lua.scope(|scope| {
            let map_table = make_map_table(&lua, scope, &cell)?;
            lua.globals().set("map", map_table)?;
            lua.load(r#"map:polyline({{0,0},{1,1}}, map:road_color())"#)
                .exec()
        })
        .expect("scope");
        assert_eq!(sink.len(), 1);
        assert_eq!(sink[0].color, DARK.road_motorway);
    }

    /// `map:point(0, 0, "x", 196)` accepts an integer as a direct
    /// xterm-256 index — the painted cell's fg must be `Indexed(196)`.
    #[test]
    fn point_accepts_integer_xterm_index() {
        let (mut buf, area, frame, theme) = fixture(40, 10);
        let mut sink: Vec<UserPolyline> = Vec::new();
        let mut api = MapApi::new(&mut buf, area, &frame, &theme, None, &mut sink);
        let lua = Lua::new();
        let cell = std::cell::RefCell::new(&mut api);
        lua.scope(|scope| {
            let map_table = make_map_table(&lua, scope, &cell)?;
            lua.globals().set("map", map_table)?;
            lua.load(r#"map:point(0, 0, "x", 196)"#).exec()
        })
        .expect("scope");
        let mut found = false;
        for x in 0..area.width {
            for y in 0..area.height {
                let cell = &buf[(x, y)];
                if cell.symbol() == "x" {
                    assert_eq!(
                        cell.style().fg,
                        Some(ratatui::style::Color::Indexed(196)),
                        "fg must be Indexed(196)"
                    );
                    found = true;
                }
            }
        }
        assert!(found, "expected 'x' cell painted somewhere");
    }

    /// `map:label(0, 0, "hi", 214)` accepts an integer colour — the
    /// painted cells' fg must all be `Indexed(214)`.
    #[test]
    fn label_accepts_integer_xterm_index() {
        let (mut buf, area, frame, theme) = fixture(40, 10);
        let mut sink: Vec<UserPolyline> = Vec::new();
        let mut api = MapApi::new(&mut buf, area, &frame, &theme, None, &mut sink);
        let lua = Lua::new();
        let cell = std::cell::RefCell::new(&mut api);
        lua.scope(|scope| {
            let map_table = make_map_table(&lua, scope, &cell)?;
            lua.globals().set("map", map_table)?;
            lua.load(r#"map:label(0, 0, "hi", 214)"#).exec()
        })
        .expect("scope");
        let mut found = false;
        for x in 0..area.width {
            for y in 0..area.height {
                let cell = &buf[(x, y)];
                if cell.symbol() == "h" || cell.symbol() == "i" {
                    assert_eq!(
                        cell.style().fg,
                        Some(ratatui::style::Color::Indexed(214)),
                        "fg must be Indexed(214)"
                    );
                    found = true;
                }
            }
        }
        assert!(found, "expected 'h' or 'i' cell painted somewhere");
    }
}
