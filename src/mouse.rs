//! Mouse input handler. Translates raw crossterm `MouseEvent`s into
//! `Core` (map state) updates and UI observer notifications.
//!
//! Flow: gate on modal widgets → update map state → notify UI
//! observers (`InfoOverlay` cursor readout). Key and mouse paths stay
//! intentionally separate — they have different semantics (keys are
//! modal/captured, mouse is observer+target) and unifying them has
//! been a documented regret in other Rust TUI apps (gitui).

use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};

use crate::app::InputEffect;
use crate::core::Core;
use crate::ui::UiState;

#[derive(Default)]
pub struct MouseHandler {
    drag_from: Option<(u16, u16)>,
}

impl MouseHandler {
    pub fn handle(&mut self, event: MouseEvent, core: &mut Core, ui: &mut UiState) -> InputEffect {
        // Search is modal — ignore mouse while its panel is open.
        if ui.focus.is_plugin("search") {
            return InputEffect::None;
        }

        let effect = self.update_core(event, core);
        ui.info.set_cursor((event.column, event.row));
        effect
    }

    fn update_core(&mut self, event: MouseEvent, core: &mut Core) -> InputEffect {
        let (cols, rows) = crossterm::terminal::size().unwrap_or((80, 24));
        let dx = event.column as f64 - cols as f64 / 2.0;
        let dy = event.row as f64 - rows as f64 / 2.0;

        match event.kind {
            MouseEventKind::Moved => InputEffect::Plugin,
            MouseEventKind::Down(MouseButton::Left) => {
                self.drag_from = Some((event.column, event.row));
                InputEffect::None
            }
            MouseEventKind::Drag(MouseButton::Left) => {
                if let Some((prev_x, prev_y)) = self.drag_from {
                    let drag_dx = event.column as i16 - prev_x as i16;
                    let drag_dy = event.row as i16 - prev_y as i16;
                    self.drag_from = Some((event.column, event.row));
                    if drag_dx != 0 || drag_dy != 0 {
                        core.pan_by_cells(drag_dx, drag_dy);
                        return InputEffect::Map;
                    }
                }
                InputEffect::None
            }
            MouseEventKind::Up(MouseButton::Left) => {
                self.drag_from = None;
                InputEffect::None
            }
            MouseEventKind::ScrollUp => {
                core.zoom_towards(dx, dy, core.zoom_step());
                InputEffect::Map
            }
            MouseEventKind::ScrollDown => {
                core.zoom_towards(dx, dy, -core.zoom_step());
                InputEffect::Map
            }
            _ => InputEffect::None,
        }
    }
}
