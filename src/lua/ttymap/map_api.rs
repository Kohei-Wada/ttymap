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
