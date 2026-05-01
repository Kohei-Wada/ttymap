//! Lua-side bridge for [`MapApi`].
//!
//! Plugin authors call `map:point(lon, lat, glyph, color)` from Lua.
//! `color` is a theme-aware string keyword today (`"accent"` |
//! `"accent_alt"`); raw integer / RGB support comes later if a
//! plugin actually needs it.
//!
//! `MapApi` carries a non-`'static` lifetime (it borrows the
//! ratatui buffer for one frame), so we can't use mlua's
//! `Scope::create_userdata_ref_mut` (which requires `T: 'static`).
//! Instead we build a per-frame Lua table whose methods are
//! `scope.create_function`-wrapped closures over a `RefCell` of the
//! `MapApi` ref. The table mimics a userdata to the script — same
//! `map:method(...)` syntax — but avoids the lifetime restriction.

use std::cell::RefCell;

use mlua::{Lua, Scope, Table};

use crate::compositor::MapApi;
use crate::compositor::map_api::Anchor;
use crate::geo::LonLat;

/// Build the Lua-facing `map` table for a single `paint_on_map`
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

/// Resolve a Lua-side colour keyword. Unknown keywords fall back to
/// `accent_color` — a typo-y plugin still renders, just in the
/// "wrong" colour, instead of crashing the script.
fn resolve_color(p: &MapApi<'_>, name: Option<&mlua::String>) -> ratatui::style::Color {
    match name.and_then(|s| s.to_str().ok()) {
        Some(s) if &*s == "accent_alt" => p.accent_alt_color(),
        Some(s) if &*s == "muted" => p.muted_color(),
        _ => p.accent_color(),
    }
}

/// Sibling of `resolve_color` that returns the xterm-256 palette
/// **index** rather than a `ratatui::style::Color`. The render thread
/// (Canvas / BrailleBuffer) speaks `u8`, while the ratatui-side
/// primitives (`point`, `label`) speak `Color`. Unwraps the
/// `Color::Indexed(u8)` variant the theme already produces; any other
/// variant falls back to white (xterm 7) — defensive only, the theme
/// always emits indexed colours today.
///
/// Supported keywords:
/// - `"road"` → `palette.road_motorway` (blends naturally with map road rendering)
/// - `"accent_alt"` → secondary accent
/// - `"muted"` → muted foreground
/// - anything else (including `"accent"`) → primary accent
fn resolve_color_xterm(p: &MapApi<'_>, name: Option<&mlua::String>) -> u8 {
    if let Some(s) = name.and_then(|s| s.to_str().ok()) {
        if &*s == "road" {
            return p.road_color_xterm();
        }
    }
    let color = match name.and_then(|s| s.to_str().ok()) {
        Some(s) if &*s == "accent_alt" => p.accent_alt_color(),
        Some(s) if &*s == "muted" => p.muted_color(),
        _ => p.accent_color(),
    };
    match color {
        ratatui::style::Color::Indexed(i) => i,
        _ => 7,
    }
}

// First param is the receiver from Lua's `map:point(...)` colon
// syntax — `map:point(a, b, c)` desugars to `map.point(map, a, b, c)`,
// so the closure sees the map table itself as its first argument.
// We discard it; the actual `MapApi` handle is the captured `cell`.
fn point(
    cell: &RefCell<&mut MapApi<'_>>,
    (_self, lon, lat, glyph, color_name): (
        mlua::Table,
        f64,
        f64,
        mlua::String,
        Option<mlua::String>,
    ),
) -> mlua::Result<()> {
    let glyph_str = glyph.to_str()?;
    let glyph_char = glyph_str.chars().next().unwrap_or(' ');
    let mut p = cell.borrow_mut();
    let color = resolve_color(&p, color_name.as_ref());
    p.point(LonLat { lon, lat }, glyph_char, color);
    Ok(())
}

// `map:text_anchored(anchor, rows_in, text, color?)` — paints `text`
// at one of the four screen-space corners. Anchor is a hyphenated
// keyword ("top-left" / "top-right" / "bottom-left" / "bottom-right");
// `rows_in` offsets toward the interior.
fn text_anchored(
    cell: &RefCell<&mut MapApi<'_>>,
    (_self, anchor, rows_in, text, color_name): (
        mlua::Table,
        mlua::String,
        u16,
        mlua::String,
        Option<mlua::String>,
    ),
) -> mlua::Result<()> {
    let anchor_str = anchor.to_str()?;
    let anchor = anchor_from_str(&anchor_str);
    let text_str = text.to_str()?;
    let mut p = cell.borrow_mut();
    let color = resolve_color(&p, color_name.as_ref());
    p.text_anchored(anchor, rows_in, &text_str, color);
    Ok(())
}

// `map:label(lon, lat, text, color?)` — paints `text` starting one
// cell to the right of the projected point. Multi-cell text picks up
// the same colour fallback as `point`.
fn label(
    cell: &RefCell<&mut MapApi<'_>>,
    (_self, lon, lat, text, color_name): (
        mlua::Table,
        f64,
        f64,
        mlua::String,
        Option<mlua::String>,
    ),
) -> mlua::Result<()> {
    let text_str = text.to_str()?;
    let mut p = cell.borrow_mut();
    let color = resolve_color(&p, color_name.as_ref());
    p.label(LonLat { lon, lat }, &text_str, color);
    Ok(())
}

// `map:polyline(coords, color?)` — coords is a Lua sequence of
// `{lon, lat}` pairs; color is one of the theme keywords accepted by
// `point`/`label`. Length-1 coords are silently dropped (matches
// `point`/`label`'s off-canvas behaviour). The polyline is queued
// for the next frame's render task — there is a 1-frame latency.
fn polyline(
    cell: &RefCell<&mut MapApi<'_>>,
    (_self, coords_table, color_name): (mlua::Table, mlua::Table, Option<mlua::String>),
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
    let color = resolve_color_xterm(&p, color_name.as_ref());
    p.push_polyline_overlay(coords, color);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compositor::MapApi;
    use crate::map::render::frame::MapFrame;
    use crate::map::render::overlay::UserPolyline;
    use crate::theme::{DARK, UiTheme};
    use mlua::Lua;
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
        drop(api);
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
        drop(api);
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
        drop(api);
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
        drop(api);
        assert_eq!(sink.len(), 1);
        assert_eq!(sink[0].color, accent_idx);
    }
}
