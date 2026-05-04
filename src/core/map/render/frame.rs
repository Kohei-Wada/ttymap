//! Map frame — structured cell data produced by the render pipeline.

use std::fmt::Write;

use crate::geo::LonLat;

/// A single terminal cell in the rendered map.
#[derive(Clone, Debug)]
pub struct MapCell {
    pub ch: char,
    pub fg: u8,
    pub bg: u8,
}

/// A complete rendered map frame (row-major grid of cells). `center` and
/// `zoom` record the view the frame was rendered at so overlays (wiki
/// markers, etc.) can project points to the same coordinates regardless
/// of how stale the frame is relative to the current app state.
#[derive(Clone, Debug)]
pub struct MapFrame {
    pub cells: Vec<MapCell>,
    pub cols: u16,
    pub rows: u16,
    pub center: LonLat,
    pub zoom: f64,
}

impl MapFrame {
    /// Serialize the frame as an xterm-256-color ANSI string.
    ///
    /// Emits `\x1b[38;5;Nm` / `\x1b[48;5;Nm` only when the fg/bg
    /// changes between adjacent cells, and `\x1b[0m\n` at the end of
    /// each row so copy-paste / stdout pipes produce a well-formed
    /// coloured grid. Used by the in-app `:export` path and by the
    /// `ttymap snap` subcommand.
    pub fn to_ansi(&self) -> String {
        let mut out = String::new();
        let cols = self.cols as usize;
        let rows = self.rows as usize;
        for row in 0..rows {
            let mut last_fg: Option<u8> = None;
            let mut last_bg: Option<u8> = None;
            for col in 0..cols {
                let Some(cell) = self.cells.get(row * cols + col) else {
                    continue;
                };
                if last_fg != Some(cell.fg) {
                    let _ = write!(out, "\x1b[38;5;{}m", cell.fg);
                    last_fg = Some(cell.fg);
                }
                if last_bg != Some(cell.bg) {
                    let _ = write!(out, "\x1b[48;5;{}m", cell.bg);
                    last_bg = Some(cell.bg);
                }
                out.push(cell.ch);
            }
            out.push_str("\x1b[0m\n");
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cell(ch: char, fg: u8, bg: u8) -> MapCell {
        MapCell { ch, fg, bg }
    }

    fn frame_2x2() -> MapFrame {
        // Two rows:
        //   row 0: 'a' fg=1 bg=0, 'b' fg=1 bg=0  (same style → fg/bg
        //          written once)
        //   row 1: 'c' fg=2 bg=0, 'd' fg=2 bg=3  (bg changes mid-row)
        MapFrame {
            cells: vec![
                cell('a', 1, 0),
                cell('b', 1, 0),
                cell('c', 2, 0),
                cell('d', 2, 3),
            ],
            cols: 2,
            rows: 2,
            center: LonLat { lon: 0.0, lat: 0.0 },
            zoom: 0.0,
        }
    }

    #[test]
    fn to_ansi_emits_fg_bg_once_per_run() {
        let ansi = frame_2x2().to_ansi();
        // row 0: fg/bg set once, then 'a' 'b', then reset + newline.
        assert_eq!(
            ansi,
            "\x1b[38;5;1m\x1b[48;5;0mab\x1b[0m\n\
             \x1b[38;5;2m\x1b[48;5;0mc\x1b[48;5;3md\x1b[0m\n"
        );
    }

    #[test]
    fn to_ansi_newline_count_matches_rows() {
        let ansi = frame_2x2().to_ansi();
        assert_eq!(ansi.matches('\n').count(), 2);
    }

    #[test]
    fn to_ansi_empty_frame() {
        let f = MapFrame {
            cells: vec![],
            cols: 0,
            rows: 0,
            center: LonLat { lon: 0.0, lat: 0.0 },
            zoom: 0.0,
        };
        assert_eq!(f.to_ansi(), "");
    }
}
