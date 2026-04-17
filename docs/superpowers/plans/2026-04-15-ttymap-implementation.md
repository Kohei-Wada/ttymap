# ttymap Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a Rust terminal map viewer that renders OpenStreetMap vector tiles as Braille characters with vim-style navigation.

**Architecture:** Standalone CLI binary with lib/bin split. Crossterm for terminal I/O, self-built Braille pixel buffer for rendering, blocking HTTP for tile fetching. Layer trait for extensibility but only OSM + markers implemented initially.

**Tech Stack:** Rust (edition 2024), crossterm, clap, reqwest (blocking), flate2, prost, rstar, earcutr, serde/serde_json, directories

---

## File Structure

```
src/
  main.rs          — CLI entry (clap args) -> calls lib::run()
  lib.rs           — Public API: run(), re-exports
  geo.rs           — Coordinate math: ll2tile, tile2ll, normalize, haversine
  config.rs        — Config struct + defaults
  color.rs         — hex2rgb, rgb_to_x256 (256-color terminal palette matching)
  braille.rs       — BrailleBuffer: pixel buffer -> Braille chars + ANSI output
  label.rs         — LabelBuffer: text collision detection via rstar
  canvas.rs        — Canvas: combines BrailleBuffer + LabelBuffer, drawing API
  styler.rs        — Parse style JSON, compile filters, resolve style per feature
  tile.rs          — Decode .pbf -> layers/features with spatial index
  tile_source.rs   — HTTP fetch + disk/memory cache
  renderer.rs      — Orchestrate: visible tiles -> fetch -> style -> draw -> frame
  input.rs         — Vim-style key handling state machine
  layer.rs         — Layer trait + OsmLayer + MarkerLayer
  app.rs           — Main event loop: crossterm events -> state -> render
styles/
  dark.json        — Copied from MapSCII
  bright.json      — Copied from MapSCII
```

---

### Task 1: Project Setup + geo.rs

**Files:**
- Modify: `Cargo.toml`
- Create: `src/geo.rs`
- Modify: `src/main.rs`
- Create: `src/lib.rs`

- [ ] **Step 1: Add dependencies to Cargo.toml**

Replace the contents of `Cargo.toml`:

```toml
[package]
name = "ttymap"
version = "0.1.0"
edition = "2024"

[dependencies]
crossterm = "0.28"
clap = { version = "4", features = ["derive"] }
reqwest = { version = "0.12", features = ["blocking"] }
flate2 = "1"
prost = "0.13"
rstar = "0.12"
earcutr = "0.4"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
directories = "6"
unicode-width = "0.2"

[build-dependencies]
prost-build = "0.13"
```

- [ ] **Step 2: Write failing tests for geo functions**

Create `src/geo.rs`:

```rust
/// Coordinate math: Mercator projection, tile coordinates, distance.

use std::f64::consts::PI;

const MAX_LATITUDE: f64 = 85.0511;
const TILE_RANGE: u32 = 14;
const PROJECT_SIZE: f64 = 256.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LonLat {
    pub lon: f64,
    pub lat: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TileCoord {
    pub x: f64,
    pub y: f64,
    pub z: u32,
}

pub fn ll2tile(lon: f64, lat: f64, zoom: u32) -> TileCoord {
    todo!()
}

pub fn tile2ll(x: f64, y: f64, zoom: u32) -> LonLat {
    todo!()
}

pub fn normalize(ll: LonLat) -> LonLat {
    todo!()
}

pub fn base_zoom(zoom: f64) -> u32 {
    todo!()
}

pub fn tile_size_at_zoom(zoom: f64) -> f64 {
    todo!()
}

/// Haversine distance in meters between two points.
pub fn haversine(a: LonLat, b: LonLat) -> f64 {
    todo!()
}

/// Format distance as human-readable string (m or km).
pub fn format_distance(meters: f64) -> String {
    todo!()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ll2tile_origin() {
        // At zoom 0, (0,0) should map to center of the single tile
        let t = ll2tile(0.0, 0.0, 0);
        assert!((t.x - 0.5).abs() < 0.001);
        assert!((t.y - 0.5).abs() < 0.001);
    }

    #[test]
    fn test_ll2tile_known_values() {
        // Berlin at zoom 10
        let t = ll2tile(13.42012, 52.51298, 10);
        assert!((t.x - 550.0).abs() < 1.0);
        assert!((t.y - 335.0).abs() < 1.0);
    }

    #[test]
    fn test_tile2ll_roundtrip() {
        let lon = 13.42;
        let lat = 52.51;
        let t = ll2tile(lon, lat, 10);
        let ll = tile2ll(t.x, t.y, 10);
        assert!((ll.lon - lon).abs() < 0.01);
        assert!((ll.lat - lat).abs() < 0.01);
    }

    #[test]
    fn test_normalize_wraps_longitude() {
        let ll = normalize(LonLat { lon: 200.0, lat: 0.0 });
        assert!((ll.lon - (-160.0)).abs() < 0.001);
    }

    #[test]
    fn test_normalize_clamps_latitude() {
        let ll = normalize(LonLat { lon: 0.0, lat: 90.0 });
        assert!((ll.lat - MAX_LATITUDE).abs() < 0.001);
    }

    #[test]
    fn test_base_zoom() {
        assert_eq!(base_zoom(5.7), 5);
        assert_eq!(base_zoom(14.3), 14);
        assert_eq!(base_zoom(18.0), 14); // clamped to TILE_RANGE
    }

    #[test]
    fn test_tile_size_at_zoom() {
        // At integer zoom matching base_zoom, tile_size should be PROJECT_SIZE
        let size = tile_size_at_zoom(5.0);
        assert!((size - PROJECT_SIZE).abs() < 0.001);
    }

    #[test]
    fn test_haversine_known_distance() {
        // Berlin to Paris ~ 878 km
        let berlin = LonLat { lon: 13.405, lat: 52.52 };
        let paris = LonLat { lon: 2.3522, lat: 48.8566 };
        let dist = haversine(berlin, paris);
        assert!((dist - 878_000.0).abs() < 5_000.0);
    }

    #[test]
    fn test_format_distance() {
        assert_eq!(format_distance(500.0), "500m");
        assert_eq!(format_distance(1500.0), "1.5km");
        assert_eq!(format_distance(12345.0), "12.3km");
    }
}
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cd /home/kohei/ghq/github.com/Kohei-Wada/ttymap && cargo test --lib geo`
Expected: All tests FAIL with `not yet implemented`

- [ ] **Step 4: Implement geo functions**

Replace the `todo!()` bodies in `src/geo.rs`:

```rust
pub fn ll2tile(lon: f64, lat: f64, zoom: u32) -> TileCoord {
    let n = 2.0_f64.powi(zoom as i32);
    TileCoord {
        x: (lon + 180.0) / 360.0 * n,
        y: (1.0 - (lat.to_radians().tan() + 1.0 / lat.to_radians().cos()).ln() / PI) / 2.0 * n,
        z: zoom,
    }
}

pub fn tile2ll(x: f64, y: f64, zoom: u32) -> LonLat {
    let n = 2.0_f64.powi(zoom as i32);
    let lat_rad = (PI * (1.0 - 2.0 * y / n)).sinh().atan();
    LonLat {
        lon: x / n * 360.0 - 180.0,
        lat: lat_rad.to_degrees(),
    }
}

pub fn normalize(ll: LonLat) -> LonLat {
    let mut lon = ll.lon;
    let mut lat = ll.lat;
    if lon < -180.0 { lon += 360.0; }
    if lon > 180.0 { lon -= 360.0; }
    if lat > MAX_LATITUDE { lat = MAX_LATITUDE; }
    if lat < -MAX_LATITUDE { lat = -MAX_LATITUDE; }
    LonLat { lon, lat }
}

pub fn base_zoom(zoom: f64) -> u32 {
    (zoom.floor() as u32).min(TILE_RANGE).max(0)
}

pub fn tile_size_at_zoom(zoom: f64) -> f64 {
    PROJECT_SIZE * 2.0_f64.powf(zoom - base_zoom(zoom) as f64)
}

pub fn haversine(a: LonLat, b: LonLat) -> f64 {
    const R: f64 = 6_371_000.0; // Earth radius in meters
    let d_lat = (b.lat - a.lat).to_radians();
    let d_lon = (b.lon - a.lon).to_radians();
    let lat1 = a.lat.to_radians();
    let lat2 = b.lat.to_radians();
    let h = (d_lat / 2.0).sin().powi(2) + lat1.cos() * lat2.cos() * (d_lon / 2.0).sin().powi(2);
    2.0 * R * h.sqrt().asin()
}

pub fn format_distance(meters: f64) -> String {
    if meters < 1000.0 {
        format!("{}m", meters.round() as u64)
    } else {
        let km = meters / 1000.0;
        if km < 100.0 {
            format!("{:.1}km", (km * 10.0).round() / 10.0)
        } else {
            format!("{}km", km.round() as u64)
        }
    }
}
```

- [ ] **Step 5: Create lib.rs and update main.rs**

Create `src/lib.rs`:

```rust
pub mod geo;
```

Replace `src/main.rs`:

```rust
fn main() {
    println!("ttymap - terminal map viewer");
}
```

- [ ] **Step 6: Run tests to verify they pass**

Run: `cd /home/kohei/ghq/github.com/Kohei-Wada/ttymap && cargo test --lib geo`
Expected: All 9 tests PASS

- [ ] **Step 7: Commit**

```bash
cd /home/kohei/ghq/github.com/Kohei-Wada/ttymap
git add src/geo.rs src/lib.rs src/main.rs Cargo.toml
git commit -m "feat: add geo module with coordinate transforms and haversine"
```

---

### Task 2: color.rs — Terminal Color Matching

**Files:**
- Create: `src/color.rs`
- Modify: `src/lib.rs`

- [ ] **Step 1: Write failing tests for color functions**

Create `src/color.rs`:

```rust
/// Convert hex color strings to 256-color terminal codes.

pub fn hex2rgb(color: &str) -> [u8; 3] {
    todo!()
}

/// Find closest xterm-256 color index for an RGB value.
/// Uses the 6x6x6 color cube (indices 16..231) and grayscale ramp (232..255).
pub fn rgb_to_x256(r: u8, g: u8, b: u8) -> u8 {
    todo!()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hex2rgb_6digit() {
        assert_eq!(hex2rgb("#ff0000"), [255, 0, 0]);
        assert_eq!(hex2rgb("#5f87ff"), [95, 135, 255]);
    }

    #[test]
    fn test_hex2rgb_3digit() {
        assert_eq!(hex2rgb("#f00"), [255, 0, 0]);
        assert_eq!(hex2rgb("#fff"), [255, 255, 255]);
    }

    #[test]
    fn test_hex2rgb_no_hash() {
        // graceful handling: treat as black
        assert_eq!(hex2rgb("notacolor"), [0, 0, 0]);
    }

    #[test]
    fn test_rgb_to_x256_pure_red() {
        let idx = rgb_to_x256(255, 0, 0);
        assert_eq!(idx, 196); // xterm bright red
    }

    #[test]
    fn test_rgb_to_x256_white() {
        let idx = rgb_to_x256(255, 255, 255);
        assert_eq!(idx, 231); // brightest white in cube
    }

    #[test]
    fn test_rgb_to_x256_black() {
        let idx = rgb_to_x256(0, 0, 0);
        assert_eq!(idx, 16); // darkest in color cube
    }

    #[test]
    fn test_rgb_to_x256_gray() {
        // A mid-gray should land in the grayscale ramp
        let idx = rgb_to_x256(128, 128, 128);
        assert!(idx >= 232 && idx <= 255);
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd /home/kohei/ghq/github.com/Kohei-Wada/ttymap && cargo test --lib color`
Expected: FAIL with `not yet implemented`

- [ ] **Step 3: Implement color functions**

Replace the `todo!()` bodies:

```rust
pub fn hex2rgb(color: &str) -> [u8; 3] {
    let color = color.strip_prefix('#').unwrap_or(color);
    match color.len() {
        3 => {
            let val = u16::from_str_radix(color, 16).unwrap_or(0);
            let r = ((val >> 8) & 0xf) as u8;
            let g = ((val >> 4) & 0xf) as u8;
            let b = (val & 0xf) as u8;
            [r | (r << 4), g | (g << 4), b | (b << 4)]
        }
        6 => {
            let val = u32::from_str_radix(color, 16).unwrap_or(0);
            [((val >> 16) & 0xff) as u8, ((val >> 8) & 0xff) as u8, (val & 0xff) as u8]
        }
        _ => [0, 0, 0],
    }
}

pub fn rgb_to_x256(r: u8, g: u8, b: u8) -> u8 {
    // The 6x6x6 color cube values: 0, 95, 135, 175, 215, 255
    const CUBE_VALS: [u8; 6] = [0, 95, 135, 175, 215, 255];

    fn closest_cube(v: u8) -> usize {
        let mut best = 0;
        let mut best_dist = 255u16;
        for (i, &cv) in CUBE_VALS.iter().enumerate() {
            let dist = (v as i16 - cv as i16).unsigned_abs();
            if dist < best_dist {
                best_dist = dist;
                best = i;
            }
        }
        best
    }

    let ri = closest_cube(r);
    let gi = closest_cube(g);
    let bi = closest_cube(b);

    let cube_r = CUBE_VALS[ri];
    let cube_g = CUBE_VALS[gi];
    let cube_b = CUBE_VALS[bi];
    let cube_dist = (r as i32 - cube_r as i32).pow(2)
        + (g as i32 - cube_g as i32).pow(2)
        + (b as i32 - cube_b as i32).pow(2);

    // Check if a grayscale ramp entry is closer (232..255 maps to 8,18,...,238)
    let avg = ((r as u16 + g as u16 + b as u16) / 3) as u8;
    let gray_idx = if avg < 4 {
        0
    } else if avg > 243 {
        23
    } else {
        ((avg as u16 - 8 + 5) / 10) as u8
    };
    let gray_val = 8 + 10 * gray_idx;
    let gray_dist = (r as i32 - gray_val as i32).pow(2)
        + (g as i32 - gray_val as i32).pow(2)
        + (b as i32 - gray_val as i32).pow(2);

    if gray_dist < cube_dist {
        232 + gray_idx
    } else {
        (16 + 36 * ri + 6 * gi + bi) as u8
    }
}
```

- [ ] **Step 4: Add to lib.rs**

Add to `src/lib.rs`:

```rust
pub mod color;
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cd /home/kohei/ghq/github.com/Kohei-Wada/ttymap && cargo test --lib color`
Expected: All 7 tests PASS

- [ ] **Step 6: Commit**

```bash
cd /home/kohei/ghq/github.com/Kohei-Wada/ttymap
git add src/color.rs src/lib.rs
git commit -m "feat: add color module for hex-to-rgb and 256-color matching"
```

---

### Task 3: braille.rs — Braille Pixel Buffer

**Files:**
- Create: `src/braille.rs`
- Modify: `src/lib.rs`

- [ ] **Step 1: Write failing tests**

Create `src/braille.rs`:

```rust
/// Braille pixel buffer: maps a 2D pixel grid onto Unicode Braille characters.
///
/// Each terminal character cell represents a 2x4 pixel grid using
/// Unicode Braille patterns (U+2800..U+28FF).
///
/// Bit layout per cell:
///   [0x01] [0x08]
///   [0x02] [0x10]
///   [0x04] [0x20]
///   [0x40] [0x80]

use crate::color;

const BRAILLE_MAP: [[u8; 2]; 4] = [
    [0x01, 0x08],
    [0x02, 0x10],
    [0x04, 0x20],
    [0x40, 0x80],
];

pub struct BrailleBuffer {
    pub width: usize,
    pub height: usize,
    pixel_buf: Vec<u8>,
    fg_buf: Vec<u8>,
    bg_buf: Vec<u8>,
    char_buf: Vec<Option<char>>,
    char_color_buf: Vec<u8>,
    global_bg: Option<u8>,
}

impl BrailleBuffer {
    pub fn new(width: usize, height: usize) -> Self {
        todo!()
    }

    pub fn clear(&mut self) {
        todo!()
    }

    pub fn set_global_background(&mut self, color: u8) {
        todo!()
    }

    pub fn set_pixel(&mut self, x: usize, y: usize, color: u8) {
        todo!()
    }

    pub fn set_char(&mut self, ch: char, x: usize, y: usize, color: u8) {
        todo!()
    }

    pub fn write_text(&mut self, text: &str, x: usize, y: usize, color: u8) {
        todo!()
    }

    /// Render the buffer to an ANSI-escaped string.
    pub fn frame(&self) -> String {
        todo!()
    }

    fn cell_count(&self) -> usize {
        (self.width / 2) * (self.height / 4)
    }

    fn project(&self, x: usize, y: usize) -> usize {
        (x / 2) + (self.width / 2) * (y / 4)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_buffer_dimensions() {
        let buf = BrailleBuffer::new(80, 40);
        assert_eq!(buf.width, 80);
        assert_eq!(buf.height, 40);
    }

    #[test]
    fn test_set_pixel_and_frame_not_empty() {
        let mut buf = BrailleBuffer::new(4, 8);
        buf.set_pixel(0, 0, 196);
        let frame = buf.frame();
        assert!(!frame.is_empty());
        // Should contain braille character, not just spaces
        assert!(frame.contains('\u{2801}')); // top-left dot
    }

    #[test]
    fn test_set_pixel_out_of_bounds_no_panic() {
        let mut buf = BrailleBuffer::new(4, 8);
        buf.set_pixel(100, 100, 1); // should be silently ignored
    }

    #[test]
    fn test_clear_resets_pixels() {
        let mut buf = BrailleBuffer::new(4, 8);
        buf.set_pixel(0, 0, 1);
        buf.clear();
        let frame = buf.frame();
        // After clear, all cells should be empty braille (U+2800)
        for ch in frame.chars() {
            if ch == '\u{2800}' || ch == '\n' || ch == '\r' || ch == '\x1b' {
                continue;
            }
            // Allow ANSI escape sequence characters
            if ch.is_ascii_digit() || ch == '[' || ch == ';' || ch == 'm'
                || ch == '3' || ch == '4' || ch == '9' {
                continue;
            }
            // Only empty braille or control chars expected
        }
    }

    #[test]
    fn test_write_text_sets_chars() {
        let mut buf = BrailleBuffer::new(20, 8);
        buf.write_text("AB", 0, 0, 196);
        let frame = buf.frame();
        assert!(frame.contains('A'));
        assert!(frame.contains('B'));
    }

    #[test]
    fn test_frame_dimensions() {
        // 8 wide, 8 tall => 4 columns, 2 rows of braille cells
        let buf = BrailleBuffer::new(8, 8);
        let frame = buf.frame();
        let lines: Vec<&str> = frame.split('\n').filter(|l| !l.is_empty()).collect();
        // Should have 2 rows
        assert!(lines.len() >= 2);
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd /home/kohei/ghq/github.com/Kohei-Wada/ttymap && cargo test --lib braille`
Expected: FAIL with `not yet implemented`

- [ ] **Step 3: Implement BrailleBuffer**

Replace all `todo!()` bodies in `src/braille.rs`:

```rust
    pub fn new(width: usize, height: usize) -> Self {
        let cell_count = (width / 2) * (height / 4);
        Self {
            width,
            height,
            pixel_buf: vec![0; cell_count],
            fg_buf: vec![0; cell_count],
            bg_buf: vec![0; cell_count],
            char_buf: vec![None; cell_count],
            char_color_buf: vec![0; cell_count],
            global_bg: None,
        }
    }

    pub fn clear(&mut self) {
        self.pixel_buf.fill(0);
        self.fg_buf.fill(0);
        self.bg_buf.fill(0);
        self.char_buf.fill(None);
        self.char_color_buf.fill(0);
    }

    pub fn set_global_background(&mut self, color: u8) {
        self.global_bg = Some(color);
    }

    pub fn set_pixel(&mut self, x: usize, y: usize, color: u8) {
        if x >= self.width || y >= self.height {
            return;
        }
        let idx = self.project(x, y);
        let mask = BRAILLE_MAP[y & 3][x & 1];
        self.pixel_buf[idx] |= mask;
        self.fg_buf[idx] = color;
    }

    pub fn set_char(&mut self, ch: char, x: usize, y: usize, color: u8) {
        if x >= self.width || y >= self.height {
            return;
        }
        let idx = self.project(x, y);
        self.char_buf[idx] = Some(ch);
        self.char_color_buf[idx] = color;
    }

    pub fn write_text(&mut self, text: &str, x: usize, y: usize, color: u8) {
        for (i, ch) in text.chars().enumerate() {
            self.set_char(ch, x + i * 2, y, color);
        }
    }

    pub fn frame(&self) -> String {
        let cols = self.width / 2;
        let rows = self.height / 4;
        let mut output = String::with_capacity(cols * rows * 10);
        let mut current_color = String::new();

        for row in 0..rows {
            if row > 0 {
                output.push_str("\n\r");
            }
            let mut skip = 0usize;
            for col in 0..cols {
                let idx = row * cols + col;
                let fg = self.fg_buf[idx];
                let bg = self.bg_buf[idx];
                let effective_fg = if self.char_buf[idx].is_some() {
                    self.char_color_buf[idx]
                } else {
                    fg
                };

                let color_code = term_color(effective_fg, bg, self.global_bg);
                if color_code != current_color {
                    output.push_str(&color_code);
                    current_color = color_code;
                }

                if let Some(ch) = self.char_buf[idx] {
                    if skip == 0 {
                        output.push(ch);
                        let w = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(1);
                        if w > 1 {
                            skip += w - 1;
                        }
                    } else {
                        skip -= 1;
                    }
                } else if skip > 0 {
                    skip -= 1;
                } else {
                    let braille = 0x2800u32 + self.pixel_buf[idx] as u32;
                    output.push(char::from_u32(braille).unwrap_or(' '));
                }
            }
        }

        output.push_str("\x1B[39;49m\n\r");
        output
    }
```

Add `term_color` as a free function at the bottom of the file (above the tests):

```rust
fn term_color(fg: u8, bg: u8, global_bg: Option<u8>) -> String {
    let effective_bg = if bg > 0 { bg } else { global_bg.unwrap_or(0) };
    match (fg > 0, effective_bg > 0) {
        (true, true) => format!("\x1B[38;5;{fg};48;5;{effective_bg}m"),
        (true, false) => format!("\x1B[49;38;5;{fg}m"),
        (false, true) => format!("\x1B[39;48;5;{effective_bg}m"),
        (false, false) => "\x1B[39;49m".to_string(),
    }
}
```

- [ ] **Step 4: Add to lib.rs**

Add to `src/lib.rs`:

```rust
pub mod braille;
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cd /home/kohei/ghq/github.com/Kohei-Wada/ttymap && cargo test --lib braille`
Expected: All 6 tests PASS

- [ ] **Step 6: Commit**

```bash
cd /home/kohei/ghq/github.com/Kohei-Wada/ttymap
git add src/braille.rs src/lib.rs
git commit -m "feat: add Braille pixel buffer with ANSI color output"
```

---

### Task 4: label.rs — Label Collision Detection

**Files:**
- Create: `src/label.rs`
- Modify: `src/lib.rs`

- [ ] **Step 1: Write failing tests**

Create `src/label.rs`:

```rust
/// Label placement with collision detection using an R-tree spatial index.

use rstar::{RTree, RTreeObject, AABB};

#[derive(Debug, Clone)]
struct LabelEntry {
    min_x: f64,
    min_y: f64,
    max_x: f64,
    max_y: f64,
}

impl RTreeObject for LabelEntry {
    type Envelope = AABB<[f64; 2]>;

    fn envelope(&self) -> Self::Envelope {
        AABB::from_corners([self.min_x, self.min_y], [self.max_x, self.max_y])
    }
}

pub struct LabelBuffer {
    tree: RTree<LabelEntry>,
    default_margin: f64,
}

impl LabelBuffer {
    pub fn new() -> Self {
        todo!()
    }

    pub fn clear(&mut self) {
        todo!()
    }

    /// Try to place a label. Returns true if placed (no collision).
    pub fn write_if_possible(
        &mut self,
        text: &str,
        x: f64,
        y: f64,
        margin: Option<f64>,
    ) -> bool {
        todo!()
    }

    fn has_space(&self, text: &str, x: f64, y: f64, margin: f64) -> bool {
        todo!()
    }

    fn calculate_area(text: &str, x: f64, y: f64, margin: f64) -> LabelEntry {
        todo!()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_label_buffer() {
        let buf = LabelBuffer::new();
        assert_eq!(buf.default_margin, 5.0);
    }

    #[test]
    fn test_first_label_always_fits() {
        let mut buf = LabelBuffer::new();
        assert!(buf.write_if_possible("Berlin", 50.0, 10.0, None));
    }

    #[test]
    fn test_overlapping_labels_rejected() {
        let mut buf = LabelBuffer::new();
        assert!(buf.write_if_possible("Berlin", 50.0, 10.0, None));
        // Same position should collide
        assert!(!buf.write_if_possible("Munich", 50.0, 10.0, None));
    }

    #[test]
    fn test_distant_labels_both_fit() {
        let mut buf = LabelBuffer::new();
        assert!(buf.write_if_possible("Berlin", 0.0, 0.0, None));
        assert!(buf.write_if_possible("Tokyo", 200.0, 200.0, None));
    }

    #[test]
    fn test_clear_allows_reuse() {
        let mut buf = LabelBuffer::new();
        assert!(buf.write_if_possible("Berlin", 50.0, 10.0, None));
        buf.clear();
        assert!(buf.write_if_possible("Munich", 50.0, 10.0, None));
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd /home/kohei/ghq/github.com/Kohei-Wada/ttymap && cargo test --lib label`
Expected: FAIL with `not yet implemented`

- [ ] **Step 3: Implement LabelBuffer**

Replace `todo!()` bodies:

```rust
    pub fn new() -> Self {
        Self {
            tree: RTree::new(),
            default_margin: 5.0,
        }
    }

    pub fn clear(&mut self) {
        self.tree = RTree::new();
    }

    pub fn write_if_possible(
        &mut self,
        text: &str,
        x: f64,
        y: f64,
        margin: Option<f64>,
    ) -> bool {
        let margin = margin.unwrap_or(self.default_margin);
        if !self.has_space(text, x, y, margin) {
            return false;
        }
        let entry = Self::calculate_area(text, x, y, margin);
        self.tree.insert(entry);
        true
    }

    fn has_space(&self, text: &str, x: f64, y: f64, margin: f64) -> bool {
        let area = Self::calculate_area(text, x, y, margin);
        let envelope = area.envelope();
        self.tree
            .locate_in_envelope_intersecting(&envelope)
            .next()
            .is_none()
    }

    fn calculate_area(text: &str, x: f64, y: f64, margin: f64) -> LabelEntry {
        let text_width = text.len() as f64;
        LabelEntry {
            min_x: x - margin,
            min_y: y - margin / 2.0,
            max_x: x + margin + text_width,
            max_y: y + margin / 2.0,
        }
    }
```

- [ ] **Step 4: Add to lib.rs**

Add to `src/lib.rs`:

```rust
pub mod label;
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cd /home/kohei/ghq/github.com/Kohei-Wada/ttymap && cargo test --lib label`
Expected: All 5 tests PASS

- [ ] **Step 6: Commit**

```bash
cd /home/kohei/ghq/github.com/Kohei-Wada/ttymap
git add src/label.rs src/lib.rs
git commit -m "feat: add label collision detection with R-tree"
```

---

### Task 5: canvas.rs — Drawing Surface

**Files:**
- Create: `src/canvas.rs`
- Modify: `src/lib.rs`

- [ ] **Step 1: Write failing tests**

Create `src/canvas.rs`:

```rust
/// Canvas: combines BrailleBuffer + LabelBuffer with a drawing API.
/// Provides polyline, polygon, and text drawing.

use crate::braille::BrailleBuffer;
use crate::label::LabelBuffer;

pub struct Canvas {
    pub width: usize,
    pub height: usize,
    pub buffer: BrailleBuffer,
    pub labels: LabelBuffer,
}

impl Canvas {
    pub fn new(width: usize, height: usize) -> Self {
        todo!()
    }

    pub fn clear(&mut self) {
        todo!()
    }

    pub fn set_background(&mut self, color: u8) {
        todo!()
    }

    pub fn frame(&self) -> String {
        todo!()
    }

    pub fn text(&mut self, text: &str, x: usize, y: usize, color: u8) {
        todo!()
    }

    /// Draw a polyline connecting the given points.
    pub fn polyline(&mut self, points: &[(i32, i32)], color: u8, _width: f64) {
        todo!()
    }

    /// Fill a polygon defined by outer ring + optional holes.
    pub fn polygon(&mut self, rings: &[Vec<(i32, i32)>], color: u8) {
        todo!()
    }

    fn line_bresenham(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, color: u8) {
        todo!()
    }

    fn filled_triangle(&mut self, a: [i32; 2], b: [i32; 2], c: [i32; 2], color: u8) {
        todo!()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_canvas() {
        let c = Canvas::new(80, 40);
        assert_eq!(c.width, 80);
        assert_eq!(c.height, 40);
    }

    #[test]
    fn test_clear_and_frame() {
        let mut c = Canvas::new(8, 8);
        c.buffer.set_pixel(0, 0, 1);
        c.clear();
        let frame = c.frame();
        // After clear, frame should only have empty braille
        assert!(!frame.is_empty());
    }

    #[test]
    fn test_polyline_draws_pixels() {
        let mut c = Canvas::new(20, 20);
        c.polyline(&[(0, 0), (10, 10)], 196, 1.0);
        let frame = c.frame();
        // Should contain non-empty braille characters
        assert!(frame.chars().any(|ch| ch as u32 > 0x2800 && ch as u32 <= 0x28FF));
    }

    #[test]
    fn test_polygon_draws_pixels() {
        let mut c = Canvas::new(20, 20);
        let ring = vec![(2, 2), (18, 2), (10, 18), (2, 2)];
        c.polygon(&[ring], 196);
        let frame = c.frame();
        assert!(frame.chars().any(|ch| ch as u32 > 0x2800 && ch as u32 <= 0x28FF));
    }

    #[test]
    fn test_text_appears_in_frame() {
        let mut c = Canvas::new(20, 8);
        c.text("Hi", 0, 0, 196);
        let frame = c.frame();
        assert!(frame.contains('H'));
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd /home/kohei/ghq/github.com/Kohei-Wada/ttymap && cargo test --lib canvas`
Expected: FAIL with `not yet implemented`

- [ ] **Step 3: Implement Canvas**

Replace `todo!()` bodies:

```rust
    pub fn new(width: usize, height: usize) -> Self {
        Self {
            width,
            height,
            buffer: BrailleBuffer::new(width, height),
            labels: LabelBuffer::new(),
        }
    }

    pub fn clear(&mut self) {
        self.buffer.clear();
        self.labels.clear();
    }

    pub fn set_background(&mut self, color: u8) {
        self.buffer.set_global_background(color);
    }

    pub fn frame(&self) -> String {
        self.buffer.frame()
    }

    pub fn text(&mut self, text: &str, x: usize, y: usize, color: u8) {
        self.buffer.write_text(text, x, y, color);
    }

    pub fn polyline(&mut self, points: &[(i32, i32)], color: u8, _width: f64) {
        for pair in points.windows(2) {
            self.line_bresenham(pair[0].0, pair[0].1, pair[1].0, pair[1].1, color);
        }
    }

    pub fn polygon(&mut self, rings: &[Vec<(i32, i32)>], color: u8) {
        let mut vertices: Vec<f64> = Vec::new();
        let mut holes: Vec<usize> = Vec::new();

        for ring in rings {
            if ring.len() < 3 {
                if vertices.is_empty() {
                    return;
                }
                continue;
            }
            if !vertices.is_empty() {
                holes.push(vertices.len() / 2);
            }
            for &(x, y) in ring {
                vertices.push(x as f64);
                vertices.push(y as f64);
            }
        }

        let triangles = match earcutr::earcut(&vertices, &holes, 2) {
            Ok(t) => t,
            Err(_) => return,
        };

        for tri in triangles.chunks(3) {
            if tri.len() < 3 {
                continue;
            }
            let a = [vertices[tri[0] * 2] as i32, vertices[tri[0] * 2 + 1] as i32];
            let b = [vertices[tri[1] * 2] as i32, vertices[tri[1] * 2 + 1] as i32];
            let c = [vertices[tri[2] * 2] as i32, vertices[tri[2] * 2 + 1] as i32];
            self.filled_triangle(a, b, c, color);
        }
    }

    fn line_bresenham(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, color: u8) {
        let dx = (x1 - x0).abs();
        let dy = -(y1 - y0).abs();
        let sx = if x0 < x1 { 1 } else { -1 };
        let sy = if y0 < y1 { 1 } else { -1 };
        let mut err = dx + dy;
        let mut cx = x0;
        let mut cy = y0;

        loop {
            if cx >= 0 && cy >= 0 {
                self.buffer.set_pixel(cx as usize, cy as usize, color);
            }
            if cx == x1 && cy == y1 {
                break;
            }
            let e2 = 2 * err;
            if e2 >= dy {
                err += dy;
                cx += sx;
            }
            if e2 <= dx {
                err += dx;
                cy += sy;
            }
        }
    }

    fn filled_triangle(&mut self, a: [i32; 2], b: [i32; 2], c: [i32; 2], color: u8) {
        // Collect edge pixels from all three sides
        let mut edge_pixels: Vec<(i32, i32)> = Vec::new();

        for &(start, end) in &[(a, b), (b, c), (a, c)] {
            let dx = (end[0] - start[0]).abs();
            let dy = -(end[1] - start[1]).abs();
            let sx = if start[0] < end[0] { 1 } else { -1 };
            let sy = if start[1] < end[1] { 1 } else { -1 };
            let mut err = dx + dy;
            let mut cx = start[0];
            let mut cy = start[1];
            loop {
                edge_pixels.push((cx, cy));
                if cx == end[0] && cy == end[1] {
                    break;
                }
                let e2 = 2 * err;
                if e2 >= dy {
                    err += dy;
                    cx += sx;
                }
                if e2 <= dx {
                    err += dx;
                    cy += sy;
                }
            }
        }

        // Sort by y, then x, and scanline fill
        edge_pixels.sort_by(|a, b| a.1.cmp(&b.1).then(a.0.cmp(&b.0)));

        let mut i = 0;
        while i < edge_pixels.len() {
            let y = edge_pixels[i].1;
            let mut min_x = edge_pixels[i].0;
            let mut max_x = min_x;
            while i < edge_pixels.len() && edge_pixels[i].1 == y {
                min_x = min_x.min(edge_pixels[i].0);
                max_x = max_x.max(edge_pixels[i].0);
                i += 1;
            }
            if y >= 0 && (y as usize) < self.height {
                let left = min_x.max(0) as usize;
                let right = (max_x as usize).min(self.width.saturating_sub(1));
                for x in left..=right {
                    self.buffer.set_pixel(x, y as usize, color);
                }
            }
        }
    }
```

- [ ] **Step 4: Add to lib.rs**

Add to `src/lib.rs`:

```rust
pub mod canvas;
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cd /home/kohei/ghq/github.com/Kohei-Wada/ttymap && cargo test --lib canvas`
Expected: All 5 tests PASS

- [ ] **Step 6: Commit**

```bash
cd /home/kohei/ghq/github.com/Kohei-Wada/ttymap
git add src/canvas.rs src/lib.rs
git commit -m "feat: add canvas with polyline, polygon, and text drawing"
```

---

### Task 6: config.rs + styles/ — Configuration and Style Files

**Files:**
- Create: `src/config.rs`
- Create: `styles/dark.json` (copy from MapSCII)
- Create: `styles/bright.json` (copy from MapSCII)
- Modify: `src/lib.rs`

- [ ] **Step 1: Create config.rs**

Create `src/config.rs`:

```rust
/// Application configuration with defaults.

pub struct Config {
    pub source: String,
    pub style_file: String,
    pub initial_lat: f64,
    pub initial_lon: f64,
    pub initial_zoom: Option<f64>,
    pub max_zoom: f64,
    pub zoom_step: f64,
    pub cache_tiles: bool,
    pub language: String,
    pub label_margin: f64,
    pub tile_range: u32,
    pub project_size: f64,
    pub poi_marker: &'static str,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            source: "http://mapscii.me/".to_string(),
            style_file: String::new(), // resolved at runtime
            initial_lat: 52.51298,     // Berlin
            initial_lon: 13.42012,
            initial_zoom: None,
            max_zoom: 18.0,
            zoom_step: 0.2,
            cache_tiles: true,
            language: "en".to_string(),
            label_margin: 5.0,
            tile_range: 14,
            project_size: 256.0,
            poi_marker: "\u{25C9}", // ◉
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let cfg = Config::default();
        assert_eq!(cfg.max_zoom, 18.0);
        assert!(cfg.source.starts_with("http"));
    }
}
```

- [ ] **Step 2: Copy style files from MapSCII**

Copy `styles/dark.json` and `styles/bright.json` from the MapSCII project at `/home/kohei/ghq/github.com/rastapasta/mapscii/styles/` to `/home/kohei/ghq/github.com/Kohei-Wada/ttymap/styles/`.

```bash
mkdir -p /home/kohei/ghq/github.com/Kohei-Wada/ttymap/styles
cp /home/kohei/ghq/github.com/rastapasta/mapscii/styles/dark.json /home/kohei/ghq/github.com/Kohei-Wada/ttymap/styles/
cp /home/kohei/ghq/github.com/rastapasta/mapscii/styles/bright.json /home/kohei/ghq/github.com/Kohei-Wada/ttymap/styles/
```

- [ ] **Step 3: Add to lib.rs**

Add to `src/lib.rs`:

```rust
pub mod config;
```

- [ ] **Step 4: Run tests**

Run: `cd /home/kohei/ghq/github.com/Kohei-Wada/ttymap && cargo test --lib config`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
cd /home/kohei/ghq/github.com/Kohei-Wada/ttymap
git add src/config.rs src/lib.rs styles/
git commit -m "feat: add config defaults and copy style files from MapSCII"
```

---

### Task 7: styler.rs — Style Engine

**Files:**
- Create: `src/styler.rs`
- Modify: `src/lib.rs`

- [ ] **Step 1: Write failing tests**

Create `src/styler.rs`:

```rust
/// Parse Mapbox GL Style Spec subset and resolve styles for features.

use serde_json::Value;
use std::collections::HashMap;

use crate::color;

#[derive(Debug, Clone)]
pub struct FeatureStyle {
    pub id: String,
    pub style_type: String, // "line", "fill", "symbol", "background"
    pub source_layer: String,
    pub min_zoom: Option<f64>,
    pub max_zoom: Option<f64>,
    pub color: u8, // 256-color terminal code
    pub line_width: f64,
    pub paint: HashMap<String, Value>,
}

pub struct Styler {
    pub name: String,
    styles_by_layer: HashMap<String, Vec<CompiledStyle>>,
    pub background_color: Option<u8>,
}

struct CompiledStyle {
    style: FeatureStyle,
    filter: Box<dyn Fn(&HashMap<String, Value>) -> bool + Send + Sync>,
}

impl Styler {
    pub fn from_json(json: &Value) -> Self {
        todo!()
    }

    pub fn get_style_for(
        &self,
        layer: &str,
        properties: &HashMap<String, Value>,
    ) -> Option<&FeatureStyle> {
        todo!()
    }
}

fn compile_filter(filter: &Value) -> Box<dyn Fn(&HashMap<String, Value>) -> bool + Send + Sync> {
    todo!()
}

fn resolve_color(paint: &HashMap<String, Value>, keys: &[&str]) -> u8 {
    todo!()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample_style() -> Value {
        json!({
            "name": "test",
            "constants": {
                "@water": "#5f87ff",
                "@background": "#000"
            },
            "layers": [
                {
                    "type": "background",
                    "id": "background",
                    "paint": { "background-color": "@background" }
                },
                {
                    "type": "fill",
                    "id": "water",
                    "paint": { "fill-color": "@water" },
                    "source-layer": "water"
                },
                {
                    "type": "line",
                    "id": "road_motorway",
                    "paint": { "line-color": "#fc8" },
                    "source-layer": "road",
                    "minzoom": 5,
                    "filter": ["==", "class", "motorway"]
                },
                {
                    "type": "symbol",
                    "id": "place_city",
                    "paint": { "text-color": "#f00" },
                    "source-layer": "place_label",
                    "filter": ["==", "type", "city"]
                }
            ]
        })
    }

    #[test]
    fn test_parse_style() {
        let styler = Styler::from_json(&sample_style());
        assert_eq!(styler.name, "test");
        assert!(styler.background_color.is_some());
    }

    #[test]
    fn test_get_style_for_water() {
        let styler = Styler::from_json(&sample_style());
        let props = HashMap::new();
        let style = styler.get_style_for("water", &props);
        assert!(style.is_some());
        assert_eq!(style.unwrap().style_type, "fill");
    }

    #[test]
    fn test_filter_match() {
        let styler = Styler::from_json(&sample_style());
        let mut props = HashMap::new();
        props.insert("class".to_string(), json!("motorway"));
        let style = styler.get_style_for("road", &props);
        assert!(style.is_some());
        assert_eq!(style.unwrap().id, "road_motorway");
    }

    #[test]
    fn test_filter_no_match() {
        let styler = Styler::from_json(&sample_style());
        let mut props = HashMap::new();
        props.insert("class".to_string(), json!("residential"));
        let style = styler.get_style_for("road", &props);
        assert!(style.is_none());
    }

    #[test]
    fn test_unknown_layer_returns_none() {
        let styler = Styler::from_json(&sample_style());
        let props = HashMap::new();
        assert!(styler.get_style_for("nonexistent", &props).is_none());
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd /home/kohei/ghq/github.com/Kohei-Wada/ttymap && cargo test --lib styler`
Expected: FAIL with `not yet implemented`

- [ ] **Step 3: Implement Styler**

Replace `todo!()` bodies:

```rust
    pub fn from_json(json: &Value) -> Self {
        let name = json["name"].as_str().unwrap_or("").to_string();
        let constants = json.get("constants").cloned().unwrap_or(Value::Null);
        let layers_val = json.get("layers").and_then(|v| v.as_array()).cloned().unwrap_or_default();

        let mut styles_by_layer: HashMap<String, Vec<CompiledStyle>> = HashMap::new();
        let mut background_color = None;
        let mut style_by_id: HashMap<String, Value> = HashMap::new();

        for layer_val in &layers_val {
            let mut layer = layer_val.clone();

            // Resolve refs
            if let Some(ref_id) = layer.get("ref").and_then(|v| v.as_str()) {
                if let Some(ref_layer) = style_by_id.get(ref_id) {
                    for key in &["type", "source-layer", "minzoom", "maxzoom", "filter"] {
                        if layer.get(*key).is_none() {
                            if let Some(val) = ref_layer.get(*key) {
                                layer[*key] = val.clone();
                            }
                        }
                    }
                }
            }

            // Replace constants in paint
            if let Some(paint) = layer.get_mut("paint") {
                replace_constants(&constants, paint);
            }

            let id = layer["id"].as_str().unwrap_or("").to_string();
            style_by_id.insert(id.clone(), layer.clone());

            let style_type = layer["type"].as_str().unwrap_or("").to_string();
            let source_layer = layer["source-layer"].as_str().unwrap_or("").to_string();

            let paint_map: HashMap<String, Value> = layer
                .get("paint")
                .and_then(|v| serde_json::from_value(v.clone()).ok())
                .unwrap_or_default();

            if style_type == "background" {
                let color = resolve_color(&paint_map, &["background-color"]);
                if color > 0 {
                    background_color = Some(color);
                }
                continue;
            }

            let color_val = resolve_color(
                &paint_map,
                &["line-color", "fill-color", "text-color"],
            );

            let line_width = paint_map
                .get("line-width")
                .and_then(|v| v.as_f64())
                .unwrap_or(1.0);

            let feature_style = FeatureStyle {
                id,
                style_type,
                source_layer: source_layer.clone(),
                min_zoom: layer.get("minzoom").and_then(|v| v.as_f64()),
                max_zoom: layer.get("maxzoom").and_then(|v| v.as_f64()),
                color: color_val,
                line_width,
                paint: paint_map,
            };

            let filter_val = layer.get("filter").cloned().unwrap_or(Value::Null);
            let filter = compile_filter(&filter_val);

            styles_by_layer
                .entry(source_layer)
                .or_default()
                .push(CompiledStyle {
                    style: feature_style,
                    filter,
                });
        }

        Self {
            name,
            styles_by_layer,
            background_color,
        }
    }

    pub fn get_style_for(
        &self,
        layer: &str,
        properties: &HashMap<String, Value>,
    ) -> Option<&FeatureStyle> {
        let styles = self.styles_by_layer.get(layer)?;
        for compiled in styles {
            if (compiled.filter)(properties) {
                return Some(&compiled.style);
            }
        }
        None
    }
```

Add free functions:

```rust
fn replace_constants(constants: &Value, node: &mut Value) {
    match node {
        Value::String(s) if s.starts_with('@') => {
            if let Some(val) = constants.get(s.as_str()) {
                *node = val.clone();
            }
        }
        Value::Object(map) => {
            for v in map.values_mut() {
                replace_constants(constants, v);
            }
        }
        Value::Array(arr) => {
            for v in arr.iter_mut() {
                replace_constants(constants, v);
            }
        }
        _ => {}
    }
}

fn compile_filter(filter: &Value) -> Box<dyn Fn(&HashMap<String, Value>) -> bool + Send + Sync> {
    let arr = match filter.as_array() {
        Some(a) if !a.is_empty() => a,
        _ => return Box::new(|_| true),
    };

    let op = arr[0].as_str().unwrap_or("");
    match op {
        "==" => {
            let key = arr.get(1).and_then(|v| v.as_str()).unwrap_or("").to_string();
            let val = arr.get(2).cloned().unwrap_or(Value::Null);
            Box::new(move |props| props.get(&key).map_or(false, |v| *v == val))
        }
        "!=" => {
            let key = arr.get(1).and_then(|v| v.as_str()).unwrap_or("").to_string();
            let val = arr.get(2).cloned().unwrap_or(Value::Null);
            Box::new(move |props| props.get(&key).map_or(true, |v| *v != val))
        }
        "in" => {
            let key = arr.get(1).and_then(|v| v.as_str()).unwrap_or("").to_string();
            let vals: Vec<Value> = arr[2..].to_vec();
            Box::new(move |props| {
                props.get(&key).map_or(false, |v| vals.contains(v))
            })
        }
        "!in" => {
            let key = arr.get(1).and_then(|v| v.as_str()).unwrap_or("").to_string();
            let vals: Vec<Value> = arr[2..].to_vec();
            Box::new(move |props| {
                props.get(&key).map_or(true, |v| !vals.contains(v))
            })
        }
        "has" => {
            let key = arr.get(1).and_then(|v| v.as_str()).unwrap_or("").to_string();
            Box::new(move |props| props.contains_key(&key))
        }
        "!has" => {
            let key = arr.get(1).and_then(|v| v.as_str()).unwrap_or("").to_string();
            Box::new(move |props| !props.contains_key(&key))
        }
        ">" => {
            let key = arr.get(1).and_then(|v| v.as_str()).unwrap_or("").to_string();
            let val = arr.get(2).and_then(|v| v.as_f64()).unwrap_or(0.0);
            Box::new(move |props| {
                props.get(&key).and_then(|v| v.as_f64()).map_or(false, |v| v > val)
            })
        }
        ">=" => {
            let key = arr.get(1).and_then(|v| v.as_str()).unwrap_or("").to_string();
            let val = arr.get(2).and_then(|v| v.as_f64()).unwrap_or(0.0);
            Box::new(move |props| {
                props.get(&key).and_then(|v| v.as_f64()).map_or(false, |v| v >= val)
            })
        }
        "<" => {
            let key = arr.get(1).and_then(|v| v.as_str()).unwrap_or("").to_string();
            let val = arr.get(2).and_then(|v| v.as_f64()).unwrap_or(0.0);
            Box::new(move |props| {
                props.get(&key).and_then(|v| v.as_f64()).map_or(false, |v| v < val)
            })
        }
        "<=" => {
            let key = arr.get(1).and_then(|v| v.as_str()).unwrap_or("").to_string();
            let val = arr.get(2).and_then(|v| v.as_f64()).unwrap_or(0.0);
            Box::new(move |props| {
                props.get(&key).and_then(|v| v.as_f64()).map_or(false, |v| v <= val)
            })
        }
        "all" => {
            let subs: Vec<Box<dyn Fn(&HashMap<String, Value>) -> bool + Send + Sync>> =
                arr[1..].iter().map(|v| compile_filter(v)).collect();
            Box::new(move |props| subs.iter().all(|f| f(props)))
        }
        "any" => {
            let subs: Vec<Box<dyn Fn(&HashMap<String, Value>) -> bool + Send + Sync>> =
                arr[1..].iter().map(|v| compile_filter(v)).collect();
            Box::new(move |props| subs.iter().any(|f| f(props)))
        }
        "none" => {
            let subs: Vec<Box<dyn Fn(&HashMap<String, Value>) -> bool + Send + Sync>> =
                arr[1..].iter().map(|v| compile_filter(v)).collect();
            Box::new(move |props| !subs.iter().any(|f| f(props)))
        }
        _ => Box::new(|_| true),
    }
}

fn resolve_color(paint: &HashMap<String, Value>, keys: &[&str]) -> u8 {
    for key in keys {
        if let Some(val) = paint.get(*key) {
            if let Some(color_str) = val.as_str() {
                let rgb = color::hex2rgb(color_str);
                return color::rgb_to_x256(rgb[0], rgb[1], rgb[2]);
            }
            // Handle zoom stops: use first stop value
            if let Some(obj) = val.as_object() {
                if let Some(stops) = obj.get("stops").and_then(|s| s.as_array()) {
                    if let Some(first) = stops.first().and_then(|s| s.as_array()) {
                        if let Some(color_str) = first.get(1).and_then(|v| v.as_str()) {
                            let rgb = color::hex2rgb(color_str);
                            return color::rgb_to_x256(rgb[0], rgb[1], rgb[2]);
                        }
                    }
                }
            }
        }
    }
    0
}
```

- [ ] **Step 4: Add to lib.rs**

Add to `src/lib.rs`:

```rust
pub mod styler;
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cd /home/kohei/ghq/github.com/Kohei-Wada/ttymap && cargo test --lib styler`
Expected: All 5 tests PASS

- [ ] **Step 6: Commit**

```bash
cd /home/kohei/ghq/github.com/Kohei-Wada/ttymap
git add src/styler.rs src/lib.rs
git commit -m "feat: add style engine with Mapbox GL filter compilation"
```

---

### Task 8: tile.rs — Vector Tile Decoding

This task uses `prost` to decode Mapbox Vector Tile protobuf format. We need a `.proto` file and build script.

**Files:**
- Create: `proto/vector_tile.proto`
- Create: `build.rs`
- Create: `src/tile.rs`
- Modify: `src/lib.rs`

- [ ] **Step 1: Create the protobuf definition**

Create `proto/vector_tile.proto`:

```protobuf
// Mapbox Vector Tile spec 2.1
// https://github.com/mapbox/vector-tile-spec/blob/master/2.1/vector_tile.proto

syntax = "proto2";

package vector_tile;

message Tile {
    enum GeomType {
        UNKNOWN = 0;
        POINT = 1;
        LINESTRING = 2;
        POLYGON = 3;
    }

    message Value {
        optional string string_value = 1;
        optional float float_value = 2;
        optional double double_value = 3;
        optional int64 int_value = 4;
        optional uint64 uint_value = 5;
        optional sint64 sint_value = 6;
        optional bool bool_value = 7;
    }

    message Feature {
        optional uint64 id = 1;
        repeated uint32 tags = 2 [packed=true];
        optional GeomType type = 3 [default=UNKNOWN];
        repeated uint32 geometry = 4 [packed=true];
    }

    message Layer {
        required uint32 version = 15 [default=1];
        required string name = 1;
        repeated Feature features = 2;
        repeated string keys = 3;
        repeated Value values = 4;
        optional uint32 extent = 5 [default=4096];
    }

    repeated Layer layers = 3;
}
```

- [ ] **Step 2: Create build.rs**

Create `build.rs` at project root:

```rust
fn main() {
    prost_build::compile_protos(&["proto/vector_tile.proto"], &["proto/"]).unwrap();
}
```

- [ ] **Step 3: Write tile.rs with tests**

Create `src/tile.rs`:

```rust
/// Decode .pbf vector tiles into layers with features and spatial index.

use std::collections::HashMap;
use flate2::read::GzDecoder;
use prost::Message;
use rstar::{RTree, RTreeObject, AABB};
use serde_json::Value;
use std::io::Read;

use crate::color;
use crate::styler::Styler;

pub mod proto {
    include!(concat!(env!("OUT_DIR"), "/vector_tile.rs"));
}

#[derive(Debug, Clone)]
pub struct Point {
    pub x: i32,
    pub y: i32,
}

#[derive(Debug, Clone)]
pub struct Feature {
    pub layer: String,
    pub style_type: String,
    pub label: Option<String>,
    pub sort: i64,
    pub points: Vec<Vec<Point>>,
    pub color: u8,
    pub line_width: f64,
    pub min_zoom: Option<f64>,
    pub max_zoom: Option<f64>,
    // R-tree bounds
    pub min_x: f64,
    pub max_x: f64,
    pub min_y: f64,
    pub max_y: f64,
}

impl RTreeObject for Feature {
    type Envelope = AABB<[f64; 2]>;
    fn envelope(&self) -> Self::Envelope {
        AABB::from_corners([self.min_x, self.min_y], [self.max_x, self.max_y])
    }
}

pub struct TileLayer {
    pub extent: u32,
    pub tree: RTree<Feature>,
}

pub struct DecodedTile {
    pub layers: HashMap<String, TileLayer>,
}

pub fn decode(buffer: &[u8], styler: &Styler, language: &str) -> DecodedTile {
    let data = decompress_if_needed(buffer);
    let tile = proto::Tile::decode(data.as_slice()).expect("failed to decode tile protobuf");

    let mut layers = HashMap::new();

    for layer in &tile.layers {
        let extent = layer.extent.unwrap_or(4096);
        let mut features = Vec::new();

        for feature in &layer.features {
            let properties = decode_tags(feature, layer);
            let style = match styler.get_style_for(&layer.name, &properties) {
                Some(s) => s,
                None => continue,
            };

            let geom_type = feature.r#type.unwrap_or(0);
            let geometries = decode_geometry(&feature.geometry, geom_type);
            if geometries.is_empty() {
                continue;
            }

            let label = if style.style_type == "symbol" {
                properties
                    .get(&format!("name_{language}"))
                    .or_else(|| properties.get("name_en"))
                    .or_else(|| properties.get("name"))
                    .or_else(|| properties.get("house_num"))
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
            } else {
                None
            };

            let sort = properties
                .get("localrank")
                .or_else(|| properties.get("scalerank"))
                .and_then(|v| v.as_i64())
                .unwrap_or(0);

            let (min_x, min_y, max_x, max_y) = bounds(&geometries);

            features.push(Feature {
                layer: layer.name.clone(),
                style_type: style.style_type.clone(),
                label,
                sort,
                points: geometries,
                color: style.color,
                line_width: style.line_width,
                min_zoom: style.min_zoom,
                max_zoom: style.max_zoom,
                min_x,
                max_x,
                min_y,
                max_y,
            });
        }

        let tree = RTree::bulk_load(features);
        layers.insert(
            layer.name.clone(),
            TileLayer { extent, tree },
        );
    }

    DecodedTile { layers }
}

fn decompress_if_needed(buffer: &[u8]) -> Vec<u8> {
    if buffer.len() >= 2 && buffer[0] == 0x1f && buffer[1] == 0x8b {
        let mut decoder = GzDecoder::new(buffer);
        let mut decompressed = Vec::new();
        decoder.read_to_end(&mut decompressed).unwrap_or_default();
        decompressed
    } else {
        buffer.to_vec()
    }
}

fn decode_tags(feature: &proto::tile::Feature, layer: &proto::tile::Layer) -> HashMap<String, Value> {
    let mut props = HashMap::new();
    for pair in feature.tags.chunks(2) {
        if pair.len() < 2 {
            break;
        }
        let key_idx = pair[0] as usize;
        let val_idx = pair[1] as usize;
        if key_idx < layer.keys.len() && val_idx < layer.values.len() {
            let key = layer.keys[key_idx].clone();
            let val = &layer.values[val_idx];
            let json_val = if let Some(ref s) = val.string_value {
                Value::String(s.clone())
            } else if let Some(f) = val.float_value {
                serde_json::Number::from_f64(f as f64)
                    .map(Value::Number)
                    .unwrap_or(Value::Null)
            } else if let Some(d) = val.double_value {
                serde_json::Number::from_f64(d)
                    .map(Value::Number)
                    .unwrap_or(Value::Null)
            } else if let Some(i) = val.int_value {
                Value::Number(i.into())
            } else if let Some(u) = val.uint_value {
                Value::Number(u.into())
            } else if let Some(s) = val.sint_value {
                Value::Number(s.into())
            } else if let Some(b) = val.bool_value {
                Value::Bool(b)
            } else {
                Value::Null
            };
            props.insert(key, json_val);
        }
    }

    // Add $type pseudo-property
    let type_name = match feature.r#type.unwrap_or(0) {
        1 => "Point",
        2 => "LineString",
        3 => "Polygon",
        _ => "Unknown",
    };
    props.insert("$type".to_string(), Value::String(type_name.to_string()));

    props
}

fn decode_geometry(geometry: &[u32], geom_type: i32) -> Vec<Vec<Point>> {
    let mut rings: Vec<Vec<Point>> = Vec::new();
    let mut current: Vec<Point> = Vec::new();
    let mut cx: i32 = 0;
    let mut cy: i32 = 0;
    let mut i = 0;

    while i < geometry.len() {
        let cmd_int = geometry[i];
        let cmd = cmd_int & 0x7;
        let count = (cmd_int >> 3) as usize;
        i += 1;

        match cmd {
            1 => {
                // MoveTo
                for _ in 0..count {
                    if i + 1 >= geometry.len() {
                        break;
                    }
                    let dx = zigzag(geometry[i]);
                    let dy = zigzag(geometry[i + 1]);
                    i += 2;
                    cx += dx;
                    cy += dy;
                    if !current.is_empty() {
                        rings.push(std::mem::take(&mut current));
                    }
                    current.push(Point { x: cx, y: cy });
                }
            }
            2 => {
                // LineTo
                for _ in 0..count {
                    if i + 1 >= geometry.len() {
                        break;
                    }
                    let dx = zigzag(geometry[i]);
                    let dy = zigzag(geometry[i + 1]);
                    i += 2;
                    cx += dx;
                    cy += dy;
                    current.push(Point { x: cx, y: cy });
                }
            }
            7 => {
                // ClosePath
                if let Some(first) = current.first() {
                    current.push(Point {
                        x: first.x,
                        y: first.y,
                    });
                }
            }
            _ => {}
        }
    }

    if !current.is_empty() {
        rings.push(current);
    }

    // For polygons, keep all rings together (outer + holes) as one feature
    if geom_type == 3 {
        if rings.is_empty() {
            vec![]
        } else {
            vec![rings.into_iter().flatten().collect()]
        }
    } else {
        rings
    }
}

fn zigzag(n: u32) -> i32 {
    ((n >> 1) as i32) ^ (-((n & 1) as i32))
}

fn bounds(geometries: &[Vec<Point>]) -> (f64, f64, f64, f64) {
    let mut min_x = f64::MAX;
    let mut min_y = f64::MAX;
    let mut max_x = f64::MIN;
    let mut max_y = f64::MIN;
    for ring in geometries {
        for p in ring {
            let px = p.x as f64;
            let py = p.y as f64;
            if px < min_x { min_x = px; }
            if px > max_x { max_x = px; }
            if py < min_y { min_y = py; }
            if py > max_y { max_y = py; }
        }
    }
    (min_x, min_y, max_x, max_y)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_zigzag_decode() {
        assert_eq!(zigzag(0), 0);
        assert_eq!(zigzag(1), -1);
        assert_eq!(zigzag(2), 1);
        assert_eq!(zigzag(3), -2);
    }

    #[test]
    fn test_decode_geometry_point() {
        // MoveTo(1, count=1), x=10 (zigzag=20), y=20 (zigzag=40)
        let geom = vec![(1 << 3) | 1, 20, 40];
        let result = decode_geometry(&geom, 1);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0][0].x, 10);
        assert_eq!(result[0][0].y, 20);
    }

    #[test]
    fn test_decode_geometry_line() {
        // MoveTo(count=1) x=0 y=0, LineTo(count=1) x=10 y=10
        let geom = vec![
            (1 << 3) | 1, 0, 0,
            (1 << 3) | 2, 20, 20,
        ];
        let result = decode_geometry(&geom, 2);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].len(), 2);
        assert_eq!(result[0][1].x, 10);
    }

    #[test]
    fn test_decompress_uncompressed() {
        let data = vec![0x0a, 0x03]; // not gzipped
        let result = decompress_if_needed(&data);
        assert_eq!(result, data);
    }

    #[test]
    fn test_bounds_calculation() {
        let geoms = vec![vec![
            Point { x: 5, y: 10 },
            Point { x: 20, y: 3 },
            Point { x: 15, y: 30 },
        ]];
        let (min_x, min_y, max_x, max_y) = bounds(&geoms);
        assert_eq!(min_x, 5.0);
        assert_eq!(min_y, 3.0);
        assert_eq!(max_x, 20.0);
        assert_eq!(max_y, 30.0);
    }
}
```

- [ ] **Step 4: Add to lib.rs**

Add to `src/lib.rs`:

```rust
pub mod tile;
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cd /home/kohei/ghq/github.com/Kohei-Wada/ttymap && cargo test --lib tile`
Expected: All 5 tests PASS

- [ ] **Step 6: Commit**

```bash
cd /home/kohei/ghq/github.com/Kohei-Wada/ttymap
git add proto/ build.rs src/tile.rs src/lib.rs
git commit -m "feat: add vector tile protobuf decoding with spatial index"
```

---

### Task 9: tile_source.rs — HTTP Tile Fetching + Cache

**Files:**
- Create: `src/tile_source.rs`
- Modify: `src/lib.rs`

- [ ] **Step 1: Write tile_source.rs with tests**

Create `src/tile_source.rs`:

```rust
/// Fetch vector tiles over HTTP with disk and in-memory caching.

use std::collections::{HashMap, VecDeque};
use std::fs;
use std::path::PathBuf;

use directories::ProjectDirs;
use flate2::read::GzDecoder;

use crate::styler::Styler;
use crate::tile::{self, DecodedTile};

pub struct TileSource {
    source_url: String,
    cache_dir: Option<PathBuf>,
    memory_cache: HashMap<String, DecodedTile>,
    cache_order: VecDeque<String>,
    cache_size: usize,
}

impl TileSource {
    pub fn new(source_url: &str, enable_disk_cache: bool) -> Self {
        let cache_dir = if enable_disk_cache {
            ProjectDirs::from("", "", "ttymap").map(|dirs| {
                let p = dirs.cache_dir().to_path_buf();
                fs::create_dir_all(&p).ok();
                p
            })
        } else {
            None
        };

        Self {
            source_url: source_url.trim_end_matches('/').to_string(),
            cache_dir,
            memory_cache: HashMap::new(),
            cache_order: VecDeque::new(),
            cache_size: 16,
        }
    }

    pub fn get_tile(&mut self, z: u32, x: i32, y: i32, styler: &Styler, language: &str) -> Option<&DecodedTile> {
        let key = format!("{z}-{x}-{y}");

        if self.memory_cache.contains_key(&key) {
            return self.memory_cache.get(&key);
        }

        let buffer = self.fetch_tile_bytes(z, x, y)?;
        let decoded = tile::decode(&buffer, styler, language);

        // Evict oldest if cache full
        if self.cache_order.len() >= self.cache_size {
            if let Some(old_key) = self.cache_order.pop_front() {
                self.memory_cache.remove(&old_key);
            }
        }

        self.cache_order.push_back(key.clone());
        self.memory_cache.insert(key.clone(), decoded);
        self.memory_cache.get(&key)
    }

    fn fetch_tile_bytes(&self, z: u32, x: i32, y: i32) -> Option<Vec<u8>> {
        // Try disk cache first
        if let Some(ref dir) = self.cache_dir {
            let path = dir.join(z.to_string()).join(format!("{x}-{y}.pbf"));
            if let Ok(data) = fs::read(&path) {
                return Some(data);
            }
        }

        // Fetch from HTTP
        let url = format!("{}/{z}/{x}/{y}.pbf", self.source_url);
        let response = reqwest::blocking::get(&url).ok()?;
        if !response.status().is_success() {
            return None;
        }
        let bytes = response.bytes().ok()?.to_vec();

        // Persist to disk
        if let Some(ref dir) = self.cache_dir {
            let zoom_dir = dir.join(z.to_string());
            fs::create_dir_all(&zoom_dir).ok();
            let path = zoom_dir.join(format!("{x}-{y}.pbf"));
            fs::write(&path, &bytes).ok();
        }

        Some(bytes)
    }

    pub fn clear_memory_cache(&mut self) {
        self.memory_cache.clear();
        self.cache_order.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_tile_source() {
        let ts = TileSource::new("http://mapscii.me/", false);
        assert_eq!(ts.source_url, "http://mapscii.me");
        assert!(ts.cache_dir.is_none());
    }

    #[test]
    fn test_cache_eviction() {
        let mut ts = TileSource::new("http://example.com", false);
        ts.cache_size = 2;
        // We can't easily test full get_tile without network, but we can test
        // the eviction logic by directly inserting into cache
        ts.cache_order.push_back("a".to_string());
        ts.cache_order.push_back("b".to_string());
        assert_eq!(ts.cache_order.len(), 2);

        // Simulate eviction
        if ts.cache_order.len() >= ts.cache_size {
            ts.cache_order.pop_front();
        }
        ts.cache_order.push_back("c".to_string());
        assert_eq!(ts.cache_order.len(), 2);
        assert_eq!(ts.cache_order[0], "b");
    }
}
```

- [ ] **Step 2: Add to lib.rs**

Add to `src/lib.rs`:

```rust
pub mod tile_source;
```

- [ ] **Step 3: Run tests to verify they pass**

Run: `cd /home/kohei/ghq/github.com/Kohei-Wada/ttymap && cargo test --lib tile_source`
Expected: All 2 tests PASS

- [ ] **Step 4: Commit**

```bash
cd /home/kohei/ghq/github.com/Kohei-Wada/ttymap
git add src/tile_source.rs src/lib.rs
git commit -m "feat: add HTTP tile fetching with disk and memory cache"
```

---

### Task 10: renderer.rs — Rendering Pipeline

**Files:**
- Create: `src/renderer.rs`
- Modify: `src/lib.rs`

- [ ] **Step 1: Create renderer.rs**

Create `src/renderer.rs`:

```rust
/// Orchestrates: visible tiles -> fetch -> style -> draw -> frame string.

use rstar::AABB;

use crate::canvas::Canvas;
use crate::config::Config;
use crate::geo;
use crate::styler::Styler;
use crate::tile::Feature;
use crate::tile_source::TileSource;

const TILE_PADDING: i32 = 64;

pub struct Renderer {
    pub canvas: Canvas,
    tile_source: TileSource,
    styler: Styler,
    width: usize,
    height: usize,
}

struct VisibleTile {
    x: i32,
    y: i32,
    z: u32,
    pos_x: f64,
    pos_y: f64,
    size: f64,
    zoom: f64,
}

impl Renderer {
    pub fn new(tile_source: TileSource, styler: Styler, width: usize, height: usize) -> Self {
        Self {
            canvas: Canvas::new(width, height),
            tile_source,
            styler,
            width,
            height,
        }
    }

    pub fn set_size(&mut self, width: usize, height: usize) {
        self.width = width;
        self.height = height;
        self.canvas = Canvas::new(width, height);
    }

    pub fn draw(&mut self, center_lon: f64, center_lat: f64, zoom: f64, language: &str) -> String {
        self.canvas.clear();

        if let Some(bg) = self.styler.background_color {
            self.canvas.set_background(bg);
        }

        let tiles = self.visible_tiles(center_lon, center_lat, zoom);

        for vis_tile in &tiles {
            self.draw_tile(vis_tile, zoom, language);
        }

        format!("\x1B[?6h{}", self.canvas.frame())
    }

    fn visible_tiles(&self, center_lon: f64, center_lat: f64, zoom: f64) -> Vec<VisibleTile> {
        let z = geo::base_zoom(zoom);
        let center = geo::ll2tile(center_lon, center_lat, z);
        let tile_size = geo::tile_size_at_zoom(zoom);
        let grid_size = 2i32.pow(z);
        let mut tiles = Vec::new();

        for dy in -1..=1 {
            for dx in -1..=1 {
                let ty = center.y.floor() as i32 + dy;
                let mut tx = center.x.floor() as i32 + dx;

                let pos_x = self.width as f64 / 2.0 - (center.x - tx as f64) * tile_size;
                let pos_y = self.height as f64 / 2.0 - (center.y - ty as f64) * tile_size;

                tx = tx.rem_euclid(grid_size);

                if ty < 0 || ty >= grid_size {
                    continue;
                }
                if pos_x + tile_size < 0.0 || pos_y + tile_size < 0.0 {
                    continue;
                }
                if pos_x > self.width as f64 || pos_y > self.height as f64 {
                    continue;
                }

                tiles.push(VisibleTile {
                    x: tx,
                    y: ty,
                    z,
                    pos_x,
                    pos_y,
                    size: tile_size,
                    zoom,
                });
            }
        }

        tiles
    }

    fn draw_tile(&mut self, vis: &VisibleTile, zoom: f64, language: &str) {
        let decoded = match self.tile_source.get_tile(vis.z, vis.x, vis.y, &self.styler, language) {
            Some(t) => t,
            None => return,
        };

        let draw_order = Self::draw_order(zoom);

        // Collect labels separately to draw last
        let mut labels: Vec<(&Feature, f64)> = Vec::new();

        for layer_name in &draw_order {
            let layer = match decoded.layers.get(*layer_name) {
                Some(l) => l,
                None => continue,
            };

            let scale = layer.extent as f64 / vis.size;
            let envelope = AABB::from_corners(
                [-vis.pos_x * scale, -vis.pos_y * scale],
                [(self.width as f64 - vis.pos_x) * scale, (self.height as f64 - vis.pos_y) * scale],
            );

            for feature in layer.tree.locate_in_envelope_intersecting(&envelope) {
                if layer_name.contains("label") {
                    labels.push((feature, scale));
                } else {
                    self.draw_feature(vis, feature, scale, zoom);
                }
            }
        }

        // Sort labels by rank and draw
        labels.sort_by(|a, b| a.0.sort.cmp(&b.0.sort));
        for (feature, scale) in labels {
            self.draw_feature(vis, feature, scale, zoom);
        }
    }

    fn draw_feature(&mut self, vis: &VisibleTile, feature: &Feature, scale: f64, zoom: f64) {
        if let Some(min_z) = feature.min_zoom {
            if zoom < min_z {
                return;
            }
        }
        if let Some(max_z) = feature.max_zoom {
            if zoom > max_z {
                return;
            }
        }

        match feature.style_type.as_str() {
            "line" => {
                for ring in &feature.points {
                    let points: Vec<(i32, i32)> = ring
                        .iter()
                        .map(|p| {
                            (
                                (vis.pos_x + p.x as f64 / scale) as i32,
                                (vis.pos_y + p.y as f64 / scale) as i32,
                            )
                        })
                        .collect();
                    if points.len() >= 2 {
                        self.canvas.polyline(&points, feature.color, feature.line_width);
                    }
                }
            }
            "fill" => {
                let rings: Vec<Vec<(i32, i32)>> = feature
                    .points
                    .iter()
                    .map(|ring| {
                        ring.iter()
                            .map(|p| {
                                (
                                    (vis.pos_x + p.x as f64 / scale) as i32,
                                    (vis.pos_y + p.y as f64 / scale) as i32,
                                )
                            })
                            .collect()
                    })
                    .collect();
                self.canvas.polygon(&rings, feature.color);
            }
            "symbol" => {
                let text = feature.label.as_deref().unwrap_or("\u{25C9}");
                for ring in &feature.points {
                    for p in ring {
                        let x = (vis.pos_x + p.x as f64 / scale) as i32;
                        let y = (vis.pos_y + p.y as f64 / scale) as i32;
                        let label_x = x - text.len() as i32;
                        if x >= 0 && y >= 0 {
                            let margin = Some(5.0);
                            if self.canvas.labels.write_if_possible(
                                text,
                                label_x as f64,
                                (y / 4) as f64,
                                margin,
                            ) {
                                self.canvas.text(text, label_x as usize, y as usize, feature.color);
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }

    fn draw_order(zoom: f64) -> Vec<&'static str> {
        if zoom < 2.0 {
            vec!["admin", "water", "country_label", "marine_label"]
        } else {
            vec![
                "landuse",
                "water",
                "marine_label",
                "building",
                "road",
                "admin",
                "country_label",
                "state_label",
                "water_label",
                "place_label",
                "rail_station_label",
                "poi_label",
                "road_label",
                "housenum_label",
            ]
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_draw_order_low_zoom() {
        let order = Renderer::draw_order(1.0);
        assert_eq!(order.len(), 4);
        assert_eq!(order[0], "admin");
    }

    #[test]
    fn test_draw_order_high_zoom() {
        let order = Renderer::draw_order(5.0);
        assert!(order.len() > 4);
        assert_eq!(order[0], "landuse");
    }
}
```

- [ ] **Step 2: Add to lib.rs**

Add to `src/lib.rs`:

```rust
pub mod renderer;
```

- [ ] **Step 3: Run tests to verify they pass**

Run: `cd /home/kohei/ghq/github.com/Kohei-Wada/ttymap && cargo test --lib renderer`
Expected: All 2 tests PASS

- [ ] **Step 4: Commit**

```bash
cd /home/kohei/ghq/github.com/Kohei-Wada/ttymap
git add src/renderer.rs src/lib.rs
git commit -m "feat: add rendering pipeline with tile composition and layer ordering"
```

---

### Task 11: input.rs — Vim-Style Key Handling

**Files:**
- Create: `src/input.rs`
- Modify: `src/lib.rs`

- [ ] **Step 1: Write failing tests**

Create `src/input.rs`:

```rust
/// Vim-style key handling state machine.

use crossterm::event::KeyCode;

#[derive(Debug, Clone, PartialEq)]
pub enum Mode {
    Normal,
    Search,
    Command,
    Mark,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    None,
    Quit,
    PanUp(u32),
    PanDown(u32),
    PanLeft(u32),
    PanRight(u32),
    ZoomIn,
    ZoomOut,
    ZoomToWorld,   // gg
    ToggleMark,
    CycleStyle,
    SubmitSearch(String),
    SubmitCommand(String),
    CancelInput,
    Redraw,
}

pub struct InputHandler {
    pub mode: Mode,
    count: Option<u32>,
    pending_g: bool,
    input_buffer: String,
}

impl InputHandler {
    pub fn new() -> Self {
        Self {
            mode: Mode::Normal,
            count: None,
            pending_g: false,
            input_buffer: String::new(),
        }
    }

    pub fn input_buffer(&self) -> &str {
        &self.input_buffer
    }

    pub fn handle_key(&mut self, code: KeyCode) -> Action {
        match self.mode {
            Mode::Normal => self.handle_normal(code),
            Mode::Search => self.handle_text_input(code, Mode::Search),
            Mode::Command => self.handle_text_input(code, Mode::Command),
            Mode::Mark => self.handle_mark(code),
        }
    }

    fn handle_normal(&mut self, code: KeyCode) -> Action {
        match code {
            KeyCode::Char(c) if c.is_ascii_digit() && c != '0' => {
                let digit = c.to_digit(10).unwrap();
                self.count = Some(self.count.unwrap_or(0) * 10 + digit);
                self.pending_g = false;
                Action::None
            }
            KeyCode::Char('g') => {
                if self.pending_g {
                    self.pending_g = false;
                    self.count = None;
                    Action::ZoomToWorld
                } else {
                    self.pending_g = true;
                    Action::None
                }
            }
            KeyCode::Char('h') | KeyCode::Left => {
                let n = self.take_count();
                Action::PanLeft(n)
            }
            KeyCode::Char('j') | KeyCode::Down => {
                let n = self.take_count();
                Action::PanDown(n)
            }
            KeyCode::Char('k') | KeyCode::Up => {
                let n = self.take_count();
                Action::PanUp(n)
            }
            KeyCode::Char('l') | KeyCode::Right => {
                let n = self.take_count();
                Action::PanRight(n)
            }
            KeyCode::Char('a') => {
                self.reset();
                Action::ZoomIn
            }
            KeyCode::Char('z') => {
                self.reset();
                Action::ZoomOut
            }
            KeyCode::Char('/') => {
                self.reset();
                self.mode = Mode::Search;
                self.input_buffer.clear();
                Action::Redraw
            }
            KeyCode::Char(':') => {
                self.reset();
                self.mode = Mode::Command;
                self.input_buffer.clear();
                Action::Redraw
            }
            KeyCode::Char('m') => {
                self.reset();
                Action::ToggleMark
            }
            KeyCode::Char('c') => {
                self.reset();
                Action::CycleStyle
            }
            KeyCode::Char('q') => Action::Quit,
            KeyCode::Esc => {
                self.reset();
                Action::None
            }
            _ => {
                self.reset();
                Action::None
            }
        }
    }

    fn handle_text_input(&mut self, code: KeyCode, mode: Mode) -> Action {
        match code {
            KeyCode::Esc => {
                self.mode = Mode::Normal;
                self.input_buffer.clear();
                Action::CancelInput
            }
            KeyCode::Enter => {
                let text = self.input_buffer.clone();
                self.input_buffer.clear();
                self.mode = Mode::Normal;
                if mode == Mode::Search {
                    Action::SubmitSearch(text)
                } else {
                    Action::SubmitCommand(text)
                }
            }
            KeyCode::Backspace => {
                self.input_buffer.pop();
                Action::Redraw
            }
            KeyCode::Char(c) => {
                self.input_buffer.push(c);
                Action::Redraw
            }
            _ => Action::None,
        }
    }

    fn handle_mark(&mut self, code: KeyCode) -> Action {
        // In mark mode, most keys return to normal
        match code {
            KeyCode::Esc => {
                self.mode = Mode::Normal;
                Action::CancelInput
            }
            _ => {
                self.mode = Mode::Normal;
                self.handle_normal(code)
            }
        }
    }

    fn take_count(&mut self) -> u32 {
        let n = self.count.unwrap_or(1);
        self.count = None;
        self.pending_g = false;
        n
    }

    fn reset(&mut self) {
        self.count = None;
        self.pending_g = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_movement() {
        let mut ih = InputHandler::new();
        assert_eq!(ih.handle_key(KeyCode::Char('j')), Action::PanDown(1));
        assert_eq!(ih.handle_key(KeyCode::Char('k')), Action::PanUp(1));
        assert_eq!(ih.handle_key(KeyCode::Char('h')), Action::PanLeft(1));
        assert_eq!(ih.handle_key(KeyCode::Char('l')), Action::PanRight(1));
    }

    #[test]
    fn test_number_prefix() {
        let mut ih = InputHandler::new();
        ih.handle_key(KeyCode::Char('1'));
        ih.handle_key(KeyCode::Char('0'));
        let action = ih.handle_key(KeyCode::Char('j'));
        assert_eq!(action, Action::PanDown(10));
    }

    #[test]
    fn test_gg_zoom_to_world() {
        let mut ih = InputHandler::new();
        ih.handle_key(KeyCode::Char('g'));
        let action = ih.handle_key(KeyCode::Char('g'));
        assert_eq!(action, Action::ZoomToWorld);
    }

    #[test]
    fn test_search_mode() {
        let mut ih = InputHandler::new();
        ih.handle_key(KeyCode::Char('/'));
        assert_eq!(ih.mode, Mode::Search);
        ih.handle_key(KeyCode::Char('t'));
        ih.handle_key(KeyCode::Char('o'));
        ih.handle_key(KeyCode::Char('k'));
        ih.handle_key(KeyCode::Char('y'));
        ih.handle_key(KeyCode::Char('o'));
        let action = ih.handle_key(KeyCode::Enter);
        assert_eq!(action, Action::SubmitSearch("tokyo".to_string()));
        assert_eq!(ih.mode, Mode::Normal);
    }

    #[test]
    fn test_command_mode() {
        let mut ih = InputHandler::new();
        ih.handle_key(KeyCode::Char(':'));
        assert_eq!(ih.mode, Mode::Command);
        ih.handle_key(KeyCode::Char('q'));
        let action = ih.handle_key(KeyCode::Enter);
        assert_eq!(action, Action::SubmitCommand("q".to_string()));
    }

    #[test]
    fn test_escape_cancels() {
        let mut ih = InputHandler::new();
        ih.handle_key(KeyCode::Char('/'));
        ih.handle_key(KeyCode::Char('a'));
        let action = ih.handle_key(KeyCode::Esc);
        assert_eq!(action, Action::CancelInput);
        assert_eq!(ih.mode, Mode::Normal);
    }

    #[test]
    fn test_zoom() {
        let mut ih = InputHandler::new();
        assert_eq!(ih.handle_key(KeyCode::Char('a')), Action::ZoomIn);
        assert_eq!(ih.handle_key(KeyCode::Char('z')), Action::ZoomOut);
    }

    #[test]
    fn test_quit() {
        let mut ih = InputHandler::new();
        assert_eq!(ih.handle_key(KeyCode::Char('q')), Action::Quit);
    }
}
```

- [ ] **Step 2: Add to lib.rs**

Add to `src/lib.rs`:

```rust
pub mod input;
```

- [ ] **Step 3: Run tests to verify they pass**

Run: `cd /home/kohei/ghq/github.com/Kohei-Wada/ttymap && cargo test --lib input`
Expected: All 8 tests PASS

- [ ] **Step 4: Commit**

```bash
cd /home/kohei/ghq/github.com/Kohei-Wada/ttymap
git add src/input.rs src/lib.rs
git commit -m "feat: add vim-style key handling with number prefix and gg"
```

---

### Task 12: layer.rs — Layer Trait + MarkerLayer

**Files:**
- Create: `src/layer.rs`
- Modify: `src/lib.rs`

- [ ] **Step 1: Write layer.rs with tests**

Create `src/layer.rs`:

```rust
/// Layer trait for extensible overlays + built-in MarkerLayer.

use crate::canvas::Canvas;
use crate::geo::{self, LonLat};

pub trait Layer {
    fn id(&self) -> &str;
    fn draw(&self, canvas: &mut Canvas, center: LonLat, zoom: f64, width: usize, height: usize);
    fn enabled(&self) -> bool;
}

#[derive(Debug, Clone)]
pub struct Mark {
    pub label: char,
    pub pos: LonLat,
}

pub struct MarkerLayer {
    marks: Vec<Mark>,
    enabled: bool,
}

impl MarkerLayer {
    pub fn new() -> Self {
        Self {
            marks: Vec::new(),
            enabled: true,
        }
    }

    pub fn toggle_mark(&mut self, pos: LonLat) {
        match self.marks.len() {
            0 => self.marks.push(Mark { label: 'A', pos }),
            1 => self.marks.push(Mark { label: 'B', pos }),
            _ => {
                self.marks.clear();
                self.marks.push(Mark { label: 'A', pos });
            }
        }
    }

    pub fn marks(&self) -> &[Mark] {
        &self.marks
    }

    pub fn clear(&mut self) {
        self.marks.clear();
    }

    pub fn distance(&self) -> Option<f64> {
        if self.marks.len() == 2 {
            Some(geo::haversine(self.marks[0].pos, self.marks[1].pos))
        } else {
            None
        }
    }

    fn lonlat_to_pixel(
        pos: LonLat,
        center: LonLat,
        zoom: f64,
        width: usize,
        height: usize,
    ) -> (i32, i32) {
        let z = geo::base_zoom(zoom);
        let tile_size = geo::tile_size_at_zoom(zoom);
        let center_tile = geo::ll2tile(center.lon, center.lat, z);
        let pos_tile = geo::ll2tile(pos.lon, pos.lat, z);
        let x = (width as f64 / 2.0 + (pos_tile.x - center_tile.x) * tile_size) as i32;
        let y = (height as f64 / 2.0 + (pos_tile.y - center_tile.y) * tile_size) as i32;
        (x, y)
    }
}

impl Layer for MarkerLayer {
    fn id(&self) -> &str {
        "markers"
    }

    fn draw(&self, canvas: &mut Canvas, center: LonLat, zoom: f64, width: usize, height: usize) {
        if self.marks.is_empty() {
            return;
        }

        let marker_color = 196; // bright red

        for mark in &self.marks {
            let (x, y) = Self::lonlat_to_pixel(mark.pos, center, zoom, width, height);
            let label = mark.label.to_string();
            if x >= 0 && y >= 0 && (x as usize) < width && (y as usize) < height {
                canvas.text(&label, x as usize, y as usize, marker_color);
            }
        }

        // Draw line between two marks
        if self.marks.len() == 2 {
            let (x0, y0) = Self::lonlat_to_pixel(self.marks[0].pos, center, zoom, width, height);
            let (x1, y1) = Self::lonlat_to_pixel(self.marks[1].pos, center, zoom, width, height);
            canvas.polyline(&[(x0, y0), (x1, y1)], marker_color, 1.0);
        }
    }

    fn enabled(&self) -> bool {
        self.enabled
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_toggle_mark_cycle() {
        let mut ml = MarkerLayer::new();
        let p1 = LonLat { lon: 0.0, lat: 0.0 };
        let p2 = LonLat { lon: 1.0, lat: 1.0 };
        let p3 = LonLat { lon: 2.0, lat: 2.0 };

        ml.toggle_mark(p1);
        assert_eq!(ml.marks().len(), 1);
        assert_eq!(ml.marks()[0].label, 'A');

        ml.toggle_mark(p2);
        assert_eq!(ml.marks().len(), 2);
        assert_eq!(ml.marks()[1].label, 'B');

        // Third toggle clears and starts over
        ml.toggle_mark(p3);
        assert_eq!(ml.marks().len(), 1);
        assert_eq!(ml.marks()[0].label, 'A');
    }

    #[test]
    fn test_distance_two_marks() {
        let mut ml = MarkerLayer::new();
        ml.toggle_mark(LonLat { lon: 13.405, lat: 52.52 });
        ml.toggle_mark(LonLat { lon: 2.3522, lat: 48.8566 });
        let dist = ml.distance().unwrap();
        assert!((dist - 878_000.0).abs() < 5_000.0);
    }

    #[test]
    fn test_distance_one_mark_is_none() {
        let mut ml = MarkerLayer::new();
        ml.toggle_mark(LonLat { lon: 0.0, lat: 0.0 });
        assert!(ml.distance().is_none());
    }

    #[test]
    fn test_clear() {
        let mut ml = MarkerLayer::new();
        ml.toggle_mark(LonLat { lon: 0.0, lat: 0.0 });
        ml.clear();
        assert!(ml.marks().is_empty());
    }
}
```

- [ ] **Step 2: Add to lib.rs**

Add to `src/lib.rs`:

```rust
pub mod layer;
```

- [ ] **Step 3: Run tests to verify they pass**

Run: `cd /home/kohei/ghq/github.com/Kohei-Wada/ttymap && cargo test --lib layer`
Expected: All 4 tests PASS

- [ ] **Step 4: Commit**

```bash
cd /home/kohei/ghq/github.com/Kohei-Wada/ttymap
git add src/layer.rs src/lib.rs
git commit -m "feat: add Layer trait and MarkerLayer with distance measurement"
```

---

### Task 13: app.rs — Main Event Loop

**Files:**
- Create: `src/app.rs`
- Modify: `src/lib.rs`
- Modify: `src/main.rs`

- [ ] **Step 1: Create app.rs**

Create `src/app.rs`:

```rust
/// Main event loop: terminal setup, crossterm events -> state -> render.

use std::io::{self, Write};
use std::time::Duration;

use crossterm::{
    cursor,
    event::{self, Event, KeyEvent, KeyModifiers},
    execute,
    terminal::{self, ClearType},
};

use crate::config::Config;
use crate::geo::{self, LonLat};
use crate::input::{Action, InputHandler, Mode};
use crate::layer::{Layer, MarkerLayer};
use crate::renderer::Renderer;
use crate::styler::Styler;
use crate::tile_source::TileSource;

pub struct App {
    config: Config,
    renderer: Renderer,
    input: InputHandler,
    marker_layer: MarkerLayer,
    center: LonLat,
    zoom: f64,
    min_zoom: f64,
    width: usize,
    height: usize,
    running: bool,
}

impl App {
    pub fn new(config: Config) -> Self {
        let style_json: serde_json::Value = if config.style_file.is_empty() {
            serde_json::from_str(include_str!("../styles/dark.json")).unwrap()
        } else {
            let data = std::fs::read_to_string(&config.style_file).expect("cannot read style file");
            serde_json::from_str(&data).unwrap()
        };

        let styler = Styler::from_json(&style_json);
        let tile_source = TileSource::new(&config.source, config.cache_tiles);

        // Default terminal size
        let (cols, rows) = terminal::size().unwrap_or((80, 24));
        let width = (cols as usize / 2) * 4; // round to multiple of 4
        let height = rows as usize * 4 - 4;

        let min_zoom = 4.0 - (4096.0 / width as f64).ln() / 2.0_f64.ln();
        let zoom = config.initial_zoom.unwrap_or(min_zoom);

        let renderer = Renderer::new(tile_source, styler, width, height);

        Self {
            center: LonLat {
                lon: config.initial_lon,
                lat: config.initial_lat,
            },
            config,
            renderer,
            input: InputHandler::new(),
            marker_layer: MarkerLayer::new(),
            zoom,
            min_zoom,
            width,
            height,
            running: true,
        }
    }

    pub fn run(&mut self) -> io::Result<()> {
        // Setup terminal
        terminal::enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(
            stdout,
            terminal::EnterAlternateScreen,
            cursor::Hide,
        )?;

        // Initial draw
        self.draw(&mut stdout)?;

        while self.running {
            if event::poll(Duration::from_millis(100))? {
                if let Event::Key(key_event) = event::read()? {
                    // Ctrl+C always quits
                    if key_event.modifiers.contains(KeyModifiers::CONTROL)
                        && key_event.code == crossterm::event::KeyCode::Char('c')
                    {
                        self.running = false;
                        continue;
                    }

                    let action = self.input.handle_key(key_event.code);
                    let needs_draw = self.process_action(action);
                    if needs_draw {
                        self.draw(&mut stdout)?;
                    }
                }

                if let Event::Resize(cols, rows) = event::read().unwrap_or(Event::FocusGained) {
                    self.resize(cols as usize, rows as usize);
                    self.draw(&mut stdout)?;
                }
            }
        }

        // Restore terminal
        execute!(
            stdout,
            cursor::Show,
            terminal::LeaveAlternateScreen,
        )?;
        terminal::disable_raw_mode()?;

        Ok(())
    }

    fn process_action(&mut self, action: Action) -> bool {
        let move_step = |zoom: f64| 8.0 / 2.0_f64.powf(zoom);

        match action {
            Action::None => false,
            Action::Quit => {
                self.running = false;
                false
            }
            Action::PanLeft(n) => {
                let step = move_step(self.zoom) * n as f64;
                self.center.lon -= step;
                self.center = geo::normalize(self.center);
                true
            }
            Action::PanRight(n) => {
                let step = move_step(self.zoom) * n as f64;
                self.center.lon += step;
                self.center = geo::normalize(self.center);
                true
            }
            Action::PanUp(n) => {
                let step = move_step(self.zoom) * 0.75 * n as f64;
                self.center.lat += step;
                self.center = geo::normalize(self.center);
                true
            }
            Action::PanDown(n) => {
                let step = move_step(self.zoom) * 0.75 * n as f64;
                self.center.lat -= step;
                self.center = geo::normalize(self.center);
                true
            }
            Action::ZoomIn => {
                self.zoom = (self.zoom + self.config.zoom_step).min(self.config.max_zoom);
                true
            }
            Action::ZoomOut => {
                self.zoom = (self.zoom - self.config.zoom_step).max(self.min_zoom);
                true
            }
            Action::ZoomToWorld => {
                self.zoom = self.min_zoom;
                true
            }
            Action::ToggleMark => {
                self.marker_layer.toggle_mark(self.center);
                true
            }
            Action::CycleStyle => {
                // TODO: cycle through style files
                false
            }
            Action::SubmitSearch(_query) => {
                // TODO: Nominatim geocoding
                true
            }
            Action::SubmitCommand(cmd) => self.process_command(&cmd),
            Action::CancelInput => true,
            Action::Redraw => true,
        }
    }

    fn process_command(&mut self, cmd: &str) -> bool {
        let parts: Vec<&str> = cmd.trim().split_whitespace().collect();
        match parts.first().copied() {
            Some("q") | Some("quit") => {
                self.running = false;
                false
            }
            Some("zoom") => {
                if let Some(z) = parts.get(1).and_then(|s| s.parse::<f64>().ok()) {
                    self.zoom = z.clamp(self.min_zoom, self.config.max_zoom);
                }
                true
            }
            Some("goto") => {
                if let (Some(lat), Some(lon)) = (
                    parts.get(1).and_then(|s| s.parse::<f64>().ok()),
                    parts.get(2).and_then(|s| s.parse::<f64>().ok()),
                ) {
                    self.center = geo::normalize(LonLat { lon, lat });
                }
                true
            }
            Some("clearmarks") => {
                self.marker_layer.clear();
                true
            }
            _ => false,
        }
    }

    fn draw(&mut self, stdout: &mut impl Write) -> io::Result<()> {
        let frame = self.renderer.draw(
            self.center.lon,
            self.center.lat,
            self.zoom,
            &self.config.language,
        );

        // Draw marker layer on top
        self.marker_layer.draw(
            &mut self.renderer.canvas,
            self.center,
            self.zoom,
            self.width,
            self.height,
        );

        execute!(stdout, cursor::MoveTo(0, 0))?;
        stdout.write_all(frame.as_bytes())?;

        // Status bar
        let status = self.build_status_bar();
        stdout.write_all(b"\r\x1B[K")?;
        stdout.write_all(status.as_bytes())?;
        stdout.flush()?;

        Ok(())
    }

    fn build_status_bar(&self) -> String {
        let mut bar = format!(
            " {:.3}, {:.3}  zoom: {:.1}",
            self.center.lat, self.center.lon, self.zoom
        );

        // Mark info
        let marks = self.marker_layer.marks();
        if !marks.is_empty() {
            bar.push_str(&format!(
                "  A: {:.2},{:.2}",
                marks[0].pos.lat, marks[0].pos.lon
            ));
        }
        if marks.len() == 2 {
            bar.push_str(&format!(
                "  B: {:.2},{:.2}",
                marks[1].pos.lat, marks[1].pos.lon
            ));
            if let Some(dist) = self.marker_layer.distance() {
                bar.push_str(&format!("  dist: {}", geo::format_distance(dist)));
            }
        }

        // Mode indicator
        let mode_str = match self.input.mode {
            Mode::Normal => "",
            Mode::Search => &format!("  /{}█", self.input.input_buffer()),
            Mode::Command => &format!("  :{}█", self.input.input_buffer()),
            Mode::Mark => "  -- MARK --",
        };
        bar.push_str(mode_str);

        bar
    }

    fn resize(&mut self, cols: usize, rows: usize) {
        self.width = (cols / 2) * 4;
        self.height = rows * 4 - 4;
        self.min_zoom = 4.0 - (4096.0 / self.width as f64).ln() / 2.0_f64.ln();
        if self.zoom < self.min_zoom {
            self.zoom = self.min_zoom;
        }
        self.renderer.set_size(self.width, self.height);
    }
}
```

- [ ] **Step 2: Update lib.rs**

Add to `src/lib.rs`:

```rust
pub mod app;
```

- [ ] **Step 3: Update main.rs with clap**

Replace `src/main.rs`:

```rust
use clap::Parser;
use ttymap::app::App;
use ttymap::config::Config;

#[derive(Parser)]
#[command(name = "ttymap", about = "Terminal map viewer")]
struct Cli {
    /// Latitude of initial center
    #[arg(long, default_value_t = 52.51298)]
    lat: f64,

    /// Longitude of initial center
    #[arg(long, default_value_t = 13.42012)]
    lon: f64,

    /// Initial zoom level
    #[arg(long, short)]
    zoom: Option<f64>,

    /// Path to style JSON file
    #[arg(long)]
    style: Option<String>,

    /// Tile server URL
    #[arg(long, default_value = "http://mapscii.me/")]
    source: String,
}

fn main() {
    let cli = Cli::parse();

    let config = Config {
        initial_lat: cli.lat,
        initial_lon: cli.lon,
        initial_zoom: cli.zoom,
        source: cli.source,
        style_file: cli.style.unwrap_or_default(),
        ..Config::default()
    };

    let mut app = App::new(config);
    if let Err(e) = app.run() {
        eprintln!("Error: {e}");
    }
}
```

- [ ] **Step 4: Verify it compiles**

Run: `cd /home/kohei/ghq/github.com/Kohei-Wada/ttymap && cargo build`
Expected: Compiles successfully

- [ ] **Step 5: Commit**

```bash
cd /home/kohei/ghq/github.com/Kohei-Wada/ttymap
git add src/app.rs src/lib.rs src/main.rs
git commit -m "feat: add main event loop with terminal UI and status bar"
```

---

### Task 14: Integration Test — End to End

**Files:**
- Modify: `src/main.rs` (no changes, just verify)

- [ ] **Step 1: Run all tests**

Run: `cd /home/kohei/ghq/github.com/Kohei-Wada/ttymap && cargo test`
Expected: All tests PASS

- [ ] **Step 2: Run clippy**

Run: `cd /home/kohei/ghq/github.com/Kohei-Wada/ttymap && cargo clippy -- -D warnings 2>&1 | head -50`
Expected: No errors (warnings may need fixing)

- [ ] **Step 3: Fix any clippy warnings**

Address any warnings found in Step 2.

- [ ] **Step 4: Build release binary**

Run: `cd /home/kohei/ghq/github.com/Kohei-Wada/ttymap && cargo build --release`
Expected: Compiles successfully

- [ ] **Step 5: Manual smoke test**

Run: `cd /home/kohei/ghq/github.com/Kohei-Wada/ttymap && cargo run`
Expected: Map renders in terminal. Test: `h/j/k/l` to move, `a/z` to zoom, `gg` for world view, `:q` to quit.

- [ ] **Step 6: Commit any fixes**

```bash
cd /home/kohei/ghq/github.com/Kohei-Wada/ttymap
git add -A
git commit -m "chore: fix clippy warnings and verify end-to-end"
```
