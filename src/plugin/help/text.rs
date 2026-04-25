//! Help text — pre-computed once at startup, shared via `Rc` with
//! every [`crate::plugin::help::component::HelpComponent`] instance.
//!
//! Stores plain segments (text / key / URL); colours are applied at
//! render time so theme switches update the display without
//! rebuilding the structure.

use crate::keymap::KeyMap;
use crate::map::Action;
use crate::plugin_api::prelude::*;

/// A coloured span of help text. Theme is applied at render time so
/// theme switches update the colors without rebuilding the help
/// structure.
enum Seg {
    Text(String),
    Key(String),
    Url(String),
}

type HelpLine = Vec<Seg>;

/// Pre-computed help text. Built once at startup from the keymap +
/// plugin metadata, shared (via `Rc`) with every
/// [`crate::plugin::help::component::HelpComponent`] instance so
/// pushes stay cheap.
pub struct HelpText {
    lines: Vec<HelpLine>,
}

impl HelpText {
    pub fn build(keymap: &KeyMap, plugin_help_entries: &[(String, String)]) -> Self {
        let mut lines: Vec<HelpLine> = vec![
            text_line(" A terminal-based map viewer — Mapbox vector tiles"),
            text_line(" rendered as Unicode Braille."),
            text_line(" Inspired by and built on ideas from mapscii:"),
            url_line("https://github.com/rastapasta/mapscii"),
            Vec::new(),
        ];
        for action in Action::all_listed() {
            let keys = keymap.keys_for(&AppMsg::Map(action.clone()));
            if !keys.is_empty() {
                lines.push(key_line(&keys.join(", "), action.label()));
            }
        }

        lines.push(Vec::new());
        lines.push(key_line("gg", "Zoom to world"));
        lines.push(key_line("Tab/S-Tab", "Cycle focus"));
        lines.push(key_line(":", "Command palette"));
        for (key, description) in plugin_help_entries {
            lines.push(key_line(key, description));
        }
        lines.push(Vec::new());
        lines.push(key_line("Drag / Scroll", "Pan / zoom (mouse)"));
        lines.push(Vec::new());
        lines.push(text_line(" Bug reports and pull requests welcome:"));
        lines.push(url_line("https://github.com/Kohei-Wada/ttymap"));

        Self { lines }
    }

    pub(super) fn rendered_lines(&self, win: &RenderWindow) -> Vec<Line> {
        let body = win.style(StyleKind::Body);
        let accent = win.style(StyleKind::Accent);
        let link = win.style(StyleKind::Link);
        self.lines
            .iter()
            .map(|segs| {
                let spans = segs
                    .iter()
                    .map(|s| match s {
                        Seg::Text(t) => Span::styled(t.clone(), body),
                        Seg::Key(k) => Span::styled(k.clone(), accent),
                        Seg::Url(u) => Span::styled(u.clone(), link),
                    })
                    .collect::<Vec<_>>();
                Line::from_spans(spans)
            })
            .collect()
    }
}

// ── Line builders ──────────────────────────────────────────────────────────────

pub(super) fn line_width(line: &Line) -> u16 {
    line.spans
        .iter()
        .map(|s| s.text.chars().count() as u16)
        .sum()
}

fn text_line(s: &str) -> HelpLine {
    vec![Seg::Text(s.to_string())]
}

fn url_line(url: &str) -> HelpLine {
    vec![Seg::Text(" ".to_string()), Seg::Url(url.to_string())]
}

fn key_line(key: &str, label: &str) -> HelpLine {
    vec![
        Seg::Text(" ".to_string()),
        Seg::Key(format!("{:<20}", key)),
        Seg::Text(format!(" {}", label)),
    ]
}
