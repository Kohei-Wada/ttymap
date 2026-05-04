//! Pure Lua-value parsers used by [`super::card_component`].
//!
//! Split out so `card_component.rs` stays focused on dispatch +
//! render orchestration. Everything in this module is a pure
//! function: input is a `mlua::Value` / `mlua::Table`, output is a
//! shaped Rust value, no state held between calls.

use crossterm::event::KeyCode;
use mlua::Table;

use crate::front::theme::StyleKind;

/// Read `spec.footer_hints` as a sequence of `{key, label}` pairs and
/// leak each pair so [`crate::core::compositor::Component::footer_hints`]
/// can hand back `&'static str` slices without per-call allocation.
/// Bounded leak: footer hints are read at panel construction. Two
/// accepted shapes per pair:
/// - `{ "Enter", "open" }` — positional 1-based array.
/// - `{ key = "Enter", label = "open" }` — named.
pub(super) fn parse_footer_hints(spec: &Table) -> Vec<(&'static str, &'static str)> {
    let Ok(list): mlua::Result<Table> = spec.get("footer_hints") else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in list.sequence_values::<mlua::Value>().flatten() {
        let mlua::Value::Table(pair) = entry else {
            continue;
        };
        let key: String = pair
            .get::<String>("key")
            .or_else(|_| pair.get::<String>(1))
            .unwrap_or_default();
        let label: String = pair
            .get::<String>("label")
            .or_else(|_| pair.get::<String>(2))
            .unwrap_or_default();
        if key.is_empty() && label.is_empty() {
            continue;
        }
        let key: &'static str = Box::leak(key.into_boxed_str());
        let label: &'static str = Box::leak(label.into_boxed_str());
        out.push((key, label));
    }
    out
}

/// Convert one Lua-returned line value into a vec of `(text, kind)`
/// spans. Bare string → single Body span. Table → array of
/// `{text, style}` records. Unknown style keywords fall back to
/// Body. Anything else stringifies to a single Body span so a buggy
/// plugin still renders.
pub(super) fn parse_line_value(value: mlua::Value) -> Vec<(String, StyleKind)> {
    match value {
        mlua::Value::String(s) => {
            let text = s.to_str().map(|c| c.to_string()).unwrap_or_default();
            vec![(text, StyleKind::Body)]
        }
        mlua::Value::Table(t) => {
            let mut spans = Vec::new();
            for pair in t.sequence_values::<mlua::Value>().flatten() {
                if let mlua::Value::Table(span_t) = pair {
                    let text: String = span_t.get("text").unwrap_or_default();
                    let style: Option<String> = span_t.get("style").ok();
                    spans.push((text, style_from_str(style.as_deref())));
                } else if let mlua::Value::String(s) = pair {
                    let text = s.to_str().map(|c| c.to_string()).unwrap_or_default();
                    spans.push((text, StyleKind::Body));
                }
            }
            if spans.is_empty() {
                spans.push((String::new(), StyleKind::Body));
            }
            spans
        }
        other => vec![(format!("{:?}", other), StyleKind::Body)],
    }
}

/// Parse one `items()[i]` value into a `Vec<Vec<Span>>` (a list of
/// lines, each a list of spans). Two accepted shapes:
///
/// - **`Vec<Line>`** — array of arrays. Each inner array goes
///   through [`parse_line_value`] for the per-span tagging.
/// - **`Line`** — bare span array (no outer wrapping). Treated as a
///   1-line item, mostly so simple plugins can write
///   `{{text=..., style=...}}` for a 1-line entry instead of
///   `{{{text=..., style=...}}}`.
pub(super) fn parse_item_value(value: mlua::Value) -> Vec<Vec<(String, StyleKind)>> {
    let mlua::Value::Table(t) = value else {
        // Stringify whatever this is into a 1-line, 1-span fallback.
        return vec![parse_line_value(value)];
    };
    // Inspect the first sequence entry to disambiguate Vec<Line>
    // (item is array-of-lines) from Line (item is array-of-spans):
    //
    // - A *line* is a Lua array of spans, so its array part is
    //   non-empty (has 1+ entries that are themselves either span
    //   tables or strings).
    // - A *span* is a Lua record with string keys (`text`, `style`)
    //   so its array part is empty.
    //
    // Sample item[0]: if it's a Table with array-len > 0, the
    // outer is Vec<Line>. Otherwise it's Line.
    let first_entry: Option<mlua::Value> = t
        .clone()
        .sequence_values::<mlua::Value>()
        .next()
        .transpose()
        .ok()
        .flatten();
    let looks_like_lines = matches!(
        &first_entry,
        Some(mlua::Value::Table(inner)) if inner.len().map(|n| n > 0).unwrap_or(false)
    );
    if looks_like_lines {
        t.sequence_values::<mlua::Value>()
            .flatten()
            .map(parse_line_value)
            .collect()
    } else {
        // 1-line item.
        vec![parse_line_value(mlua::Value::Table(t))]
    }
}

/// Map a Lua-side style keyword to a [`StyleKind`]. Unknown values
/// fall back to `Body` so a typo paints in the default colour rather
/// than breaking the plugin.
pub(super) fn style_from_str(name: Option<&str>) -> StyleKind {
    match name {
        Some("muted") => StyleKind::Muted,
        Some("accent") => StyleKind::Accent,
        Some("highlight") => StyleKind::Highlight,
        Some("selected") => StyleKind::Selected,
        Some("muted_fg") => StyleKind::MutedFg,
        Some("link") => StyleKind::Link,
        _ => StyleKind::Body,
    }
}

/// What the Lua `handle_event` handler asked the host to do.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum KeyAction {
    /// Pass the event to the base layer (Component default).
    Ignore,
    /// Pop the component off the stack.
    Close,
    /// Treat the event as handled with no further action.
    Consume,
}

impl KeyAction {
    pub(super) fn from_lua_return(value: mlua::Value) -> Self {
        match value {
            mlua::Value::Nil => KeyAction::Consume,
            mlua::Value::Table(t) => {
                if t.get::<bool>("close").unwrap_or(false) {
                    KeyAction::Close
                } else if t.get::<bool>("ignore").unwrap_or(false) {
                    KeyAction::Ignore
                } else {
                    KeyAction::Consume
                }
            }
            _ => KeyAction::Consume,
        }
    }
}

/// Translate a crossterm `KeyCode` into the Lua-side `code` string
/// plus, for `Char(c)`, the actual character. Unknown variants
/// surface as `"Other"` so a Lua handler can at least see the event
/// arrived without reaching for the full crossterm vocabulary.
pub(super) fn key_code_to_lua(code: KeyCode) -> (&'static str, Option<char>) {
    match code {
        KeyCode::Char(c) => ("Char", Some(c)),
        KeyCode::Enter => ("Enter", None),
        KeyCode::Esc => ("Esc", None),
        KeyCode::Tab => ("Tab", None),
        KeyCode::BackTab => ("BackTab", None),
        KeyCode::Backspace => ("Backspace", None),
        KeyCode::Up => ("Up", None),
        KeyCode::Down => ("Down", None),
        KeyCode::Left => ("Left", None),
        KeyCode::Right => ("Right", None),
        KeyCode::Home => ("Home", None),
        KeyCode::End => ("End", None),
        KeyCode::PageUp => ("PageUp", None),
        KeyCode::PageDown => ("PageDown", None),
        KeyCode::Delete => ("Delete", None),
        KeyCode::Insert => ("Insert", None),
        _ => ("Other", None),
    }
}
