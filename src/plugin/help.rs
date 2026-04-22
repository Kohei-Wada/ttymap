//! Help widget — displays keybinding help as a center overlay.
//!
//! Under the compositor model: ephemeral component, fresh instance
//! on every push. Any key closes it.

use crossterm::event::{KeyEvent, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::{Alignment, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};

use crate::app::AppMsg;
use crate::compositor::{
    Activation, Component, Context, EventResult, PaletteEntry, PaletteKind, Registrar,
};
use crate::keymap::KeyMap;
use crate::map::Action;
use crate::theme::UiTheme;

/// A colored span of help text. Theme is applied at render time so
/// theme switches update the colors without rebuilding the help
/// structure.
enum Seg {
    Text(String),
    Key(String),
    Url(String),
}

type HelpLine = Vec<Seg>;

/// Pre-computed help text. Built once at startup from the keymap +
/// plugin metadata, shared (via `Rc`) with every `HelpComponent`
/// instance so pushes stay cheap.
pub struct HelpText {
    lines: Vec<HelpLine>,
}

impl HelpText {
    pub fn build(
        keymap: &KeyMap,
        plugin_help_entries: &[(String, String)],
    ) -> Self {
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

    fn rendered_lines<'a>(&'a self, theme: &UiTheme) -> Vec<Line<'a>> {
        self.lines
            .iter()
            .map(|segs| {
                let spans: Vec<Span<'a>> = segs
                    .iter()
                    .map(|s| match s {
                        Seg::Text(t) => Span::styled(t.as_str(), theme.text()),
                        Seg::Key(k) => Span::styled(k.as_str(), theme.accent_style()),
                        Seg::Url(u) => Span::styled(u.as_str(), theme.link()),
                    })
                    .collect();
                Line::from(spans)
            })
            .collect()
    }
}

pub struct HelpComponent {
    text: std::rc::Rc<HelpText>,
}

impl HelpComponent {
    pub fn new(text: std::rc::Rc<HelpText>) -> Self {
        Self { text }
    }
}

impl Component for HelpComponent {
    fn handle_event(&mut self, _event: KeyEvent, _ctx: &Context) -> EventResult {
        // Help is fully modal: any key closes the panel.
        EventResult::Close(Vec::new())
    }

    fn render(&self, f: &mut Frame, map_inner: Rect, theme: &UiTheme) {
        if map_inner.width < 20 || map_inner.height < 10 {
            return;
        }

        let rendered = self.text.rendered_lines(theme);

        let content_width = rendered
            .iter()
            .map(|l| l.width() as u16)
            .max()
            .unwrap_or(30)
            + 6;
        let content_height = rendered.len() as u16 + 2;

        let max_width = map_inner.width.saturating_sub(4).max(20);
        let max_height = map_inner.height.saturating_sub(2).max(10);
        let popup_width = content_width.clamp(50, max_width);
        let popup_height = content_height.min(max_height);

        let x = map_inner.x + (map_inner.width - popup_width) / 2;
        let y = map_inner.y + (map_inner.height - popup_height) / 2;

        let area = Rect::new(x, y, popup_width, popup_height);
        f.render_widget(Clear, area);

        let block = theme.panel("help").title_alignment(Alignment::Center);
        let widget = Paragraph::new(rendered).style(theme.text()).block(block);
        f.render_widget(widget, area);
    }

    fn footer_hints(&self) -> Vec<(&'static str, &'static str)> {
        vec![("any key", "close")]
    }
}

/// Register the help plugin. Takes pre-computed help entries from
/// sibling plugins (harvested by the composition root) so help
/// remains in sync with what's actually loaded.
pub fn register(help_text: std::rc::Rc<HelpText>, r: &mut Registrar) {
    {
        let text = help_text.clone();
        r.add_activation(Activation {
            code: crossterm::event::KeyCode::Char('?'),
            modifiers: KeyModifiers::NONE,
            spawn: Box::new(move |_ctx: &Context| -> Box<dyn Component> {
                Box::new(HelpComponent::new(text.clone()))
            }),
        });
    }
    {
        let text = help_text;
        r.add_palette_entry(PaletteEntry {
            label: "Toggle help".to_string(),
            hint: "?".to_string(),
            kind: PaletteKind::Spawn(Box::new(move |_ctx: &Context| -> Box<dyn Component> {
                Box::new(HelpComponent::new(text.clone()))
            })),
        });
    }
}

// ── Line builders ──────────────────────────────────────────────────────────────

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
