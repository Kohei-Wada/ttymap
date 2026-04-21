//! Mouse adapter — translates raw crossterm `MouseEvent`s into a
//! sequence of `AppCommand`s for the main loop to dispatch.
//!
//! *Adapter* (not a router): wraps a device API
//! (`crossterm::MouseEvent`) and produces a different shape
//! (`AppCommand`), in the GoF sense. It does not decide where the
//! output goes — every command flows straight to
//! `app_command::dispatch`. Hit-testing against individual surfaces
//! (click inside palette popup vs. click on the map) is future work;
//! when added, a separate routing step would sit between this
//! adapter and `dispatch`, matching the cached-`Rect` + `contains`
//! pattern used by helix, zellij, bottom, and ratatui's own
//! recommendation.
//!
//! Owns cross-event drag state (`drag_from`) because translating a
//! `Drag` event into `PanCells(dx, dy)` requires knowing the
//! previous position. That state is *this adapter's translation
//! concern* (protocol-level event correlation, same role as
//! zellij's `mouse_old_event`), not something the broader app cares
//! about, so it stays encapsulated here. If drag state ever starts
//! holding *semantic* information (world-space anchor, selected
//! feature, …) it should move to the relevant domain type — helix
//! puts its `mouse_down_range: Range` on `Editor` for that reason.
//!
//! Key and mouse paths are intentionally not symmetric (see
//! [`super::router`] for the axes). Both emit the same `AppCommand`
//! vocabulary on the output side: every event emits a leading
//! `Ui(CursorMoved)`; drag additionally emits `Map(PanCells)`;
//! scroll emits `Map(ZoomAt { ... })`.

use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};

use crate::app_command::AppCommand;
use crate::map::Action;
use crate::ui::action::UiAction;

#[derive(Default)]
pub struct MouseAdapter {
    drag_from: Option<(u16, u16)>,
}

impl MouseAdapter {
    /// Translate a raw mouse event into zero or more `AppCommand`s.
    /// Every event emits a leading `Ui(CursorMoved)` for the overlay
    /// readout; the `resolve` stage appends any additional command
    /// (drag → pan, scroll → zoom).
    pub fn translate(&mut self, event: MouseEvent) -> Vec<AppCommand> {
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
