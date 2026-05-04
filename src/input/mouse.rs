//! Mouse adapter — translates raw crossterm `MouseEvent`s into a
//! sequence of `UserIntent`s for the main loop to dispatch.
//!
//! *Adapter* (not a router): wraps a device API
//! (`crossterm::MouseEvent`) and produces a different shape
//! (`UserIntent`), in the GoF sense. It does not decide where the
//! output goes — every message flows straight to `App::dispatch`.
//! Hit-testing against individual surfaces (click inside palette
//! popup vs. click on the map) is future work; when added, a
//! separate routing step would sit between this adapter and
//! `dispatch`, matching the cached-`Rect` + `contains` pattern used
//! by helix, zellij, bottom, and ratatui's own recommendation.
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
//! Key and mouse paths are intentionally not symmetric: keyboard
//! routes through the [`compositor`](crate::compositor) for focus
//! delivery before producing a `UserIntent`, while mouse is a pure
//! translator with no focus involvement. Both emit the same `UserIntent`
//! vocabulary on the output side: every event emits a leading
//! `CursorMoved`; drag additionally emits `Map(PanCells)`; scroll
//! emits `Map(ZoomAt { ... })`.

use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};

use crate::frontend::UserIntent;
use crate::map::MapAction;

#[derive(Default)]
pub struct MouseAdapter {
    drag_from: Option<(u16, u16)>,
}

impl MouseAdapter {
    /// Translate a raw mouse event into zero or more `UserIntent`s.
    /// Every event emits a leading `Ui(CursorMoved)` for the overlay
    /// readout; the `resolve` stage appends any additional message
    /// (drag → pan, scroll → zoom).
    pub fn translate(&mut self, event: MouseEvent) -> Vec<UserIntent> {
        let mut msgs = vec![UserIntent::CursorMoved(event.column, event.row)];
        if let Some(msg) = self.resolve(event) {
            msgs.push(msg);
        }
        msgs
    }

    fn resolve(&mut self, event: MouseEvent) -> Option<UserIntent> {
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
                        return Some(UserIntent::Map(MapAction::PanCells(dx, dy)));
                    }
                }
                None
            }
            MouseEventKind::Up(MouseButton::Left) => {
                self.drag_from = None;
                None
            }
            MouseEventKind::ScrollUp => Some(UserIntent::Map(MapAction::ZoomAt {
                anchor_dx,
                anchor_dy,
                zoom_in: true,
            })),
            MouseEventKind::ScrollDown => Some(UserIntent::Map(MapAction::ZoomAt {
                anchor_dx,
                anchor_dy,
                zoom_in: false,
            })),
            _ => None,
        }
    }
}
