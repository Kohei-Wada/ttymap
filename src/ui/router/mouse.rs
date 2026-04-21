//! Mouse router — translates raw crossterm `MouseEvent`s into a
//! sequence of `AppCommand`s for the main loop to dispatch.
//!
//! Sibling of [`super::route_key`] under `ui::router`. Key and mouse
//! paths stay intentionally separate — they have different semantics
//! (keys are modal/captured, mouse is observer+target) and unifying
//! them has been a documented regret in other Rust TUI apps (gitui).
//! They share the `AppCommand` vocabulary on the output side: every
//! event emits a leading `Ui(CursorMoved)`; drag additionally emits
//! `Map(PanCells)`; scroll emits `Map(ZoomAt { ... })`.
//!
//! Unlike [`super::route_key`], this is stateful (drag tracking) so
//! it lives as a `MouseRouter` struct owned by `App`. The router
//! never touches `UiState` directly — cursor-readout updates flow
//! through `AppCommand::Ui(UiAction::CursorMoved)` like every other
//! user-intent state change.

use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};

use crate::app_command::AppCommand;
use crate::map::Action;
use crate::ui::action::UiAction;

#[derive(Default)]
pub struct MouseRouter {
    drag_from: Option<(u16, u16)>,
}

impl MouseRouter {
    /// Translate a raw mouse event into zero or more `AppCommand`s.
    /// Every event emits a leading `Ui(CursorMoved)` for the overlay
    /// readout; the `resolve` stage appends any additional command
    /// (drag → pan, scroll → zoom).
    pub fn route_mouse(&mut self, event: MouseEvent) -> Vec<AppCommand> {
        let mut cmds = vec![AppCommand::Ui(UiAction::CursorMoved(
            event.column,
            event.row,
        ))];
        if let Some(cmd) = self.resolve(event) {
            cmds.push(cmd);
        }
        cmds
    }

    fn resolve(&mut self, event: MouseEvent) -> Option<AppCommand> {
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
                        return Some(AppCommand::Map(Action::PanCells(dx, dy)));
                    }
                }
                None
            }
            MouseEventKind::Up(MouseButton::Left) => {
                self.drag_from = None;
                None
            }
            MouseEventKind::ScrollUp => Some(AppCommand::Map(Action::ZoomAt {
                anchor_dx,
                anchor_dy,
                zoom_in: true,
            })),
            MouseEventKind::ScrollDown => Some(AppCommand::Map(Action::ZoomAt {
                anchor_dx,
                anchor_dy,
                zoom_in: false,
            })),
            _ => None,
        }
    }
}
