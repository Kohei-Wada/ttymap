//! Render subsystem — transforms tile data into terminal output.

pub mod braille;
pub mod canvas;
pub mod frame;
pub mod label;
pub mod panic_silence;
pub mod pipeline;
pub mod renderer;
pub mod thread;

/// Padding (in pixels) around the viewport for clipping and point filtering.
pub const VIEWPORT_PADDING: i32 = 64;

/// Calculate canvas pixel dimensions from terminal character dimensions.
/// Each terminal cell = 2 pixels wide × 4 pixels tall (Braille).
/// Accounts for map border (2 cols, 2 rows) and footer (1 row).
pub fn canvas_size(cols: u16, rows: u16) -> (usize, usize) {
    let inner_cols = cols.saturating_sub(2) as usize;
    let inner_rows = rows.saturating_sub(3) as usize; // 2 border + 1 footer
    let width = (inner_cols / 2) * 4;
    let height = inner_rows * 4;
    (width.max(4), height.max(4))
}
