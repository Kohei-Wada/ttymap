//! Mouse input handler. Translates raw crossterm `MouseEvent`s into
//! `Command`s (and a cursor-tracking side effect), then hands them
//! back to the main loop for `command::dispatch`.
//!
//! Key and mouse paths stay intentionally separate — they have
//! different semantics (keys are modal/captured, mouse is
//! observer+target) and unifying them has been a documented regret in
//! other Rust TUI apps (gitui). But they now share the same `Command`
//! vocabulary on the output side: drag → `Command::Map(PanCells)`,
//! scroll → `Command::Map(ZoomAt { ... })`.

use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};

use crate::command::Command;
use crate::map::Action;
use crate::ui::UiState;

#[derive(Default)]
pub struct MouseHandler {
    drag_from: Option<(u16, u16)>,
}

impl MouseHandler {
    /// Consume a raw mouse event. Updates cursor readout on
    /// `InfoOverlay` as a side effect, and returns an optional
    /// `Command` for the main loop to dispatch. `None` means "handled
    /// locally (cursor move, click tracking, modal gate) — no state
    /// change for the dispatcher".
    pub fn handle(&mut self, event: MouseEvent, ui: &mut UiState) -> Option<Command> {
        // Search is modal — ignore mouse while its panel is open.
        if ui.focus.is_plugin("search") {
            return None;
        }

        ui.info.set_cursor((event.column, event.row));
        self.resolve(event)
    }

    fn resolve(&mut self, event: MouseEvent) -> Option<Command> {
        let (cols, rows) = crossterm::terminal::size().unwrap_or((80, 24));
        let anchor_dx = event.column as f64 - cols as f64 / 2.0;
        let anchor_dy = event.row as f64 - rows as f64 / 2.0;

        match event.kind {
            MouseEventKind::Moved => None,
            MouseEventKind::Down(MouseButton::Left) => {
                self.drag_from = Some((event.column, event.row));
                None
            }
            MouseEventKind::Drag(MouseButton::Left) => {
                if let Some((prev_x, prev_y)) = self.drag_from {
                    let dx = event.column as i16 - prev_x as i16;
                    let dy = event.row as i16 - prev_y as i16;
                    self.drag_from = Some((event.column, event.row));
                    if dx != 0 || dy != 0 {
                        return Some(Command::Map(Action::PanCells(dx, dy)));
                    }
                }
                None
            }
            MouseEventKind::Up(MouseButton::Left) => {
                self.drag_from = None;
                None
            }
            MouseEventKind::ScrollUp => Some(Command::Map(Action::ZoomAt {
                anchor_dx,
                anchor_dy,
                zoom_in: true,
            })),
            MouseEventKind::ScrollDown => Some(Command::Map(Action::ZoomAt {
                anchor_dx,
                anchor_dy,
                zoom_in: false,
            })),
            _ => None,
        }
    }
}
