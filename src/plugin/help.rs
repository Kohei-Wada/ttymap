//! Help widget — displays keybinding help as a center overlay.

use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::{Alignment, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};

use crate::app_command::{AppCommand, Effect, FocusSurface, SurfaceCtx};
use crate::keymap::KeyMap;
use crate::map::Action;
use crate::theme::UiTheme;

use super::Plugin;

/// A colored span of help text. Theme is applied at render time so
/// theme switches update the colors without rebuilding the help
/// structure.
enum Seg {
    /// Plain prose text.
    Text(String),
    /// Key name / binding (e.g. "h", "Tab", "gg"). Rendered with the
    /// accent color so bindings stand out from their descriptions.
    Key(String),
    /// URL that modern terminals auto-detect for click-to-open.
    /// Underlined to hint at clickability.
    Url(String),
}

type HelpLine = Vec<Seg>;

#[derive(Default)]
pub struct HelpPlugin {
    active: bool,
    lines: Vec<HelpLine>,
}

impl HelpPlugin {
    pub fn new() -> Self {
        Self::default()
    }

    /// Build the help text. `other_plugins` is inspected for each
    /// plugin's activation keys + description, so the listing stays in
    /// sync with the plugins actually loaded rather than a hardcoded
    /// table in this file. Help includes its own entry automatically.
    pub fn build(&mut self, keymap: &KeyMap, other_plugins: &[&dyn Plugin]) {
        let entries: Vec<(String, String)> = plugin_entries(self)
            .into_iter()
            .chain(other_plugins.iter().flat_map(|p| plugin_entries(*p)))
            .collect();

        let mut lines: Vec<HelpLine> = vec![
            text_line(" A terminal-based map viewer — Mapbox vector tiles"),
            text_line(" rendered as Unicode Braille."),
            text_line(" Inspired by and built on ideas from mapscii:"),
            url_line("https://github.com/rastapasta/mapscii"),
            Vec::new(),
        ];
        for action in Action::all_listed() {
            let keys = keymap.keys_for(&AppCommand::Map(action.clone()));
            if !keys.is_empty() {
                lines.push(key_line(&keys.join(", "), action.label()));
            }
        }

        lines.push(Vec::new());
        lines.push(key_line("gg", "Zoom to world"));
        lines.push(key_line("Tab/S-Tab", "Cycle focus"));
        lines.push(key_line(":", "Command palette"));
        for (key, description) in &entries {
            lines.push(key_line(key, description));
        }
        lines.push(Vec::new());
        lines.push(key_line("Drag / Scroll", "Pan / zoom (mouse)"));
        lines.push(Vec::new());
        lines.push(text_line(" Bug reports and pull requests welcome:"));
        lines.push(url_line("https://github.com/Kohei-Wada/ttymap"));

        self.lines = lines;
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

impl Plugin for HelpPlugin {
    fn tag(&self) -> &str {
        "help"
    }

    fn description(&self) -> &str {
        "Toggle help"
    }

    fn activation_keys(&self) -> Vec<&'static str> {
        vec!["?"]
    }

    fn activate(&mut self, _ctx: SurfaceCtx) {
        self.active = true;
    }

    fn deactivate(&mut self) {
        // Modal: losing focus means closing.
        self.active = false;
    }

    fn render(&self, f: &mut Frame, map_inner: Rect, theme: &UiTheme) {
        if map_inner.width < 20 || map_inner.height < 10 {
            return;
        }

        let rendered = self.rendered_lines(theme);

        // Fit content with breathing room, but cap at ~80% of the map
        // area so the popup doesn't dominate the viewport.
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
}

/// Help is fully modal: any key closes the panel. The host notices
/// `is_visible()=false` and releases focus accordingly.
impl FocusSurface for HelpPlugin {
    fn handle_key(
        &mut self,
        _code: KeyCode,
        _modifiers: KeyModifiers,
        _ctx: SurfaceCtx,
    ) -> Effect {
        self.active = false;
        Effect::Consumed
    }

    fn is_visible(&self) -> bool {
        self.active
    }

    fn footer_hints(&self) -> Vec<(&'static str, &'static str)> {
        vec![("any key", "close")]
    }
}

/// `(activation_key, description)` pairs from one plugin. Empty
/// description means the plugin opted out of help listing.
fn plugin_entries(p: &dyn Plugin) -> Vec<(String, String)> {
    let desc = p.description();
    if desc.is_empty() {
        return Vec::new();
    }
    p.activation_keys()
        .into_iter()
        .map(|k| (k.to_string(), desc.to_string()))
        .collect()
}

// ── Line builders ──────────────────────────────────────────────────────────────

fn text_line(s: &str) -> HelpLine {
    vec![Seg::Text(s.to_string())]
}

fn url_line(url: &str) -> HelpLine {
    vec![Seg::Text(" ".to_string()), Seg::Url(url.to_string())]
}

/// `" <key padded to 20>  <label>"` — matches the original plain-text
/// layout but splits the key into its own span so it can be colored.
fn key_line(key: &str, label: &str) -> HelpLine {
    vec![
        Seg::Text(" ".to_string()),
        Seg::Key(format!("{:<20}", key)),
        Seg::Text(format!(" {}", label)),
    ]
}
