//! Palette data — `ColorPalette` struct + `DARK` / `BRIGHT` consts.
//!
//! Values are xterm-256 colour indices (`u8`). No ratatui dependency,
//! so both the map renderer (via `styler::Styler`) and the UI layer
//! (via [`crate::theme::UiTheme`]) read from here.

/// All colours used by a single theme.
pub struct ColorPalette {
    // background
    pub background: u8,
    // shared (used by both map renderer and UI chrome)
    pub accent: u8,
    pub accent_alt: u8,
    pub fg: u8,
    pub muted: u8,
    // natural
    pub water: u8,
    pub waterway: u8,
    pub waterway_deep: u8,
    pub landuse_park: u8,
    pub landuse_wood: u8,
    pub landuse_cemetery: u8,
    pub landuse_hospital: u8,
    pub landuse_school: u8,
    pub landuse_overlay: u8,
    // structures
    pub building: u8,
    pub aeroway: u8,
    // roads
    pub road_motorway: u8,
    pub road_trunk_primary: u8,
    pub road_street: u8,
    pub road_path_pedestrian: u8,
    pub road_rail: u8,
    // tunnels
    pub tunnel_motorway: u8,
    pub tunnel_link: u8,
    // admin boundaries
    pub admin_level_2: u8,
    pub admin_level_3: u8,
    pub admin_level_4: u8,
    pub admin_disputed: u8,
    pub admin_maritime_2: u8,
    pub admin_maritime_3: u8,
    // labels
    pub place_city: u8,
    pub place_town: u8,
    pub place_village: u8,
    pub place_other: u8,
    pub marine_label: u8,
    pub water_label: u8,
    pub poi_label_1: u8,
    pub poi_label_2: u8,
    pub poi_label_3: u8,
    pub poi_label_4: u8,
    pub rail_station_label: u8,
    pub airport_label: u8,
    pub road_label: u8,
    pub housenum_label: u8,
}

/// Parse a hex color string (`#RGB` or `#RRGGBB`) into an RGB array.
/// Returns `[0, 0, 0]` for invalid input.
#[cfg(test)]
pub(crate) fn hex2rgb(color: &str) -> [u8; 3] {
    fn parse(color: &str) -> Option<[u8; 3]> {
        let s = color.trim_start_matches('#');
        match s.len() {
            3 => {
                let r = u8::from_str_radix(&s[0..1], 16).ok()?;
                let g = u8::from_str_radix(&s[1..2], 16).ok()?;
                let b = u8::from_str_radix(&s[2..3], 16).ok()?;
                Some([r * 17, g * 17, b * 17])
            }
            6 => {
                let r = u8::from_str_radix(&s[0..2], 16).ok()?;
                let g = u8::from_str_radix(&s[2..4], 16).ok()?;
                let b = u8::from_str_radix(&s[4..6], 16).ok()?;
                Some([r, g, b])
            }
            _ => None,
        }
    }
    parse(color).unwrap_or([0, 0, 0])
}

/// Find the closest xterm-256 color index for a given RGB value.
/// Considers the 6x6x6 color cube (16..=231) and grayscale ramp (232..=255).
pub(crate) fn rgb_to_x256(r: u8, g: u8, b: u8) -> u8 {
    const CUBE_LEVELS: [u8; 6] = [0, 95, 135, 175, 215, 255];

    fn nearest_cube_index(v: u8) -> usize {
        let mut best_idx = 0usize;
        let mut best_dist = u32::MAX;
        for (i, &level) in CUBE_LEVELS.iter().enumerate() {
            let d = (v as i32 - level as i32).unsigned_abs();
            if d < best_dist {
                best_dist = d;
                best_idx = i;
            }
        }
        best_idx
    }

    let ri = nearest_cube_index(r);
    let gi = nearest_cube_index(g);
    let bi = nearest_cube_index(b);

    let cube_r = CUBE_LEVELS[ri] as i32;
    let cube_g = CUBE_LEVELS[gi] as i32;
    let cube_b = CUBE_LEVELS[bi] as i32;

    let cube_dist =
        (r as i32 - cube_r).pow(2) + (g as i32 - cube_g).pow(2) + (b as i32 - cube_b).pow(2);
    let cube_idx = 16 + 36 * ri as u8 + 6 * gi as u8 + bi as u8;

    fn nearest_gray_index(v: u8) -> usize {
        let mut best_idx = 0usize;
        let mut best_dist = u32::MAX;
        for i in 0..24usize {
            let level = 8 + 10 * i as i32;
            let d = (v as i32 - level).unsigned_abs();
            if d < best_dist {
                best_dist = d;
                best_idx = i;
            }
        }
        best_idx
    }

    let gray_avg = (r as i32 + g as i32 + b as i32) / 3;
    let gi_idx = nearest_gray_index(gray_avg as u8);
    let gray_level = 8 + 10 * gi_idx as i32;

    let gray_dist = (r as i32 - gray_level).pow(2)
        + (g as i32 - gray_level).pow(2)
        + (b as i32 - gray_level).pow(2);
    let gray_idx = 232 + gi_idx as u8;

    if gray_dist < cube_dist {
        gray_idx
    } else {
        cube_idx
    }
}

/// Approximate the RGB value of an xterm-256 palette index.
///
/// Mapping (matches the canonical xterm-256 layout):
/// - 0..=15: standard colours — uses a fixed table for the 16 base
///   colours. The exact RGB values are terminal-defined; we use the
///   widely-shared "VGA-ish" defaults so subsequent darkening
///   produces predictable results.
/// - 16..=231: 6×6×6 colour cube. `idx - 16` decodes into r/g/b
///   indices 0..5, each mapped through `[0, 95, 135, 175, 215, 255]`.
/// - 232..=255: 24-step grayscale ramp at `8 + 10 * (idx - 232)`.
pub(crate) fn xterm_to_rgb(idx: u8) -> [u8; 3] {
    const CUBE_LEVELS: [u8; 6] = [0, 95, 135, 175, 215, 255];
    // Standard 16 base colours, VGA-style approximations. Used for
    // indices 0..=15 (system colours; the exact RGB depends on the
    // user's terminal palette settings, but this covers the common
    // case well enough for our darkening lookup).
    const BASE_16: [[u8; 3]; 16] = [
        [0, 0, 0],
        [128, 0, 0],
        [0, 128, 0],
        [128, 128, 0],
        [0, 0, 128],
        [128, 0, 128],
        [0, 128, 128],
        [192, 192, 192],
        [128, 128, 128],
        [255, 0, 0],
        [0, 255, 0],
        [255, 255, 0],
        [0, 0, 255],
        [255, 0, 255],
        [0, 255, 255],
        [255, 255, 255],
    ];
    let idx = idx as usize;
    if idx < 16 {
        BASE_16[idx]
    } else if idx < 232 {
        let cube = idx - 16;
        let r = cube / 36;
        let g = (cube / 6) % 6;
        let b = cube % 6;
        [CUBE_LEVELS[r], CUBE_LEVELS[g], CUBE_LEVELS[b]]
    } else {
        let gray = 8u16 + 10 * (idx - 232) as u16;
        let v = gray.min(255) as u8;
        [v, v, v]
    }
}

/// Step down each RGB channel by N cube levels (`[0, 95, 135, 175,
/// 215, 255]`), clamped at level 0. Preserves hue better than RGB
/// halving + nearest-cube quantisation: the channel ratios remain
/// monotonic in cube space.
fn step_down_to_cube(value: u8, steps: usize) -> u8 {
    const LEVELS: [u8; 6] = [0, 95, 135, 175, 215, 255];
    // Nearest cube index for `value`.
    let mut nearest = 0usize;
    let mut best = u32::MAX;
    for (i, &lvl) in LEVELS.iter().enumerate() {
        let d = (value as i32 - lvl as i32).unsigned_abs();
        if d < best {
            best = d;
            nearest = i;
        }
    }
    let target = nearest.saturating_sub(steps);
    LEVELS[target]
}

/// Return an xterm-256 index that's perceptually darker than `idx`
/// while preserving its hue. Used by the user-overlay third pass to
/// render saturated punched cells (water/forest/…) at a brightness
/// matching the surrounding `⣿` glyphs.
///
/// Stepping down by 2 cube levels per channel is empirically about
/// half-brightness for typical map colours; halving raw RGB and
/// rounding to the nearest cube produces the same darkness but
/// drifts the hue (`yellow` → `olive`, etc.) because the cube has
/// only 6 levels per channel and rounding can land on a different
/// channel ratio.
///
/// `Modifier::DIM` (ANSI `\x1b[2m`) doesn't work for this path: that
/// attribute is foreground-only in essentially every terminal, so
/// the bg-fill of a saturated punched cell stays at full brightness
/// and outshines its `⣿` neighbours. Darkening the colour itself
/// before storing it in `bg_buf` / `fg_buf` is the only portable
/// way to reduce perceived brightness.
pub(crate) fn dim_xterm(idx: u8) -> u8 {
    const STEP_DOWN: usize = 2;
    let [r, g, b] = xterm_to_rgb(idx);
    let dr = step_down_to_cube(r, STEP_DOWN);
    let dg = step_down_to_cube(g, STEP_DOWN);
    let db = step_down_to_cube(b, STEP_DOWN);
    rgb_to_x256(dr, dg, db)
}

pub const DARK: ColorPalette = ColorPalette {
    background: 16,            // #000
    accent: 226,               // Yellow (#ff0)
    accent_alt: 14,            // Cyan
    fg: 231,                   // White
    muted: 242,                // DarkGray
    water: 69,                 // #5f87ff
    waterway: 153,             // #a0c8f0
    waterway_deep: 153,        // same as waterway in dark
    landuse_park: 107,         // #7b5
    landuse_wood: 71,          // #6a4
    landuse_cemetery: 254,     // #e0e4dd
    landuse_hospital: 175,     // #d9b
    landuse_school: 255,       // #f0e8f8
    landuse_overlay: 188,      // #d8e8c8
    building: 103,             // #99b
    aeroway: 255,              // #f0ede9
    road_motorway: 222,        // #fc8
    road_trunk_primary: 229,   // #fea
    road_street: 231,          // #fff
    road_path_pedestrian: 181, // #cba
    road_rail: 250,            // #bbb
    tunnel_motorway: 223,      // #ffdaa6
    tunnel_link: 230,          // #fff4c6
    admin_level_2: 231,        // #fff
    admin_level_3: 146,        // #aac
    admin_level_4: 146,        // same as level_3 in dark
    admin_disputed: 231,       // #fff
    admin_maritime_2: 111,     // #9bf
    admin_maritime_3: 111,     // #8af
    place_city: 196,           // #f00
    place_town: 167,           // #d33
    place_village: 167,        // #c33
    place_other: 131,          // #b33
    marine_label: 110,         // #74aee9
    water_label: 110,          // #74aee9
    poi_label_1: 226,          // #ff0
    poi_label_2: 226,          // #ee0
    poi_label_3: 184,          // #cc0
    poi_label_4: 142,          // #aa0
    rail_station_label: 241,   // #666
    airport_label: 241,        // #666
    road_label: 95,            // #765
    housenum_label: 103,       // #88a
};

/// Bright theme — white background, dark foreground colors.
pub const BRIGHT: ColorPalette = ColorPalette {
    background: 231,           // white
    accent: 25,                // dark blue (#005faf)
    accent_alt: 160,           // dark red (#d70000) — pops against accent blue
    fg: 16,                    // black
    muted: 245,                // mid gray
    water: 69,                 // #5f87ff (same as dark)
    waterway: 74,              // steel blue (#5fafd7)
    waterway_deep: 25,         // dark blue (#005faf)
    landuse_park: 71,          // green (#5faf5f)
    landuse_wood: 28,          // dark green (#008700)
    landuse_cemetery: 249,     // light gray (#b2b2b2)
    landuse_hospital: 217,     // light pink (#ffafaf)
    landuse_school: 183,       // light lavender (#d7afff)
    landuse_overlay: 151,      // sage (#afd7af)
    building: 249,             // light gray (#b2b2b2)
    aeroway: 250,              // silver (#bcbcbc)
    road_motorway: 166,        // dark orange (#d75f00)
    road_trunk_primary: 166, // dark orange — matches motorway in bright (was 130 brown, never visible)
    road_street: 245,        // mid gray (#8a8a8a)
    road_path_pedestrian: 138, // rosy brown (#af8787)
    road_rail: 240,          // dark gray (#585858)
    tunnel_motorway: 166,    // dark orange — uniform with road in bright
    tunnel_link: 166,        // dark orange — uniform with road in bright
    admin_level_2: 238,      // dark gray (#444444)
    admin_level_3: 243,      // mid gray (#767676)
    admin_level_4: 247,      // light gray (#9e9e9e)
    admin_disputed: 243,     // mid gray, dashed feel
    admin_maritime_2: 67,    // slate blue (#5f87af)
    admin_maritime_3: 103,   // dull blue (#8787af)
    place_city: 160,         // dark red (#d70000)
    place_town: 130,         // brown (#af5f00)
    place_village: 95,       // mauve (#875f5f)
    place_other: 242,        // gray (#6c6c6c)
    marine_label: 67,        // slate blue (#5f87af)
    water_label: 67,         // slate blue
    poi_label_1: 130,        // brown (#af5f00)
    poi_label_2: 137,        // tan (#af875f)
    poi_label_3: 101,        // olive (#87875f)
    poi_label_4: 243,        // gray (#767676)
    rail_station_label: 90,  // plum (#870087... actually #8700d7 area)
    airport_label: 25,       // dark blue (#005faf)
    road_label: 242,         // gray (#6c6c6c)
    housenum_label: 247,     // light gray (#9e9e9e)
};

#[cfg(test)]
mod tests {
    use super::*;

    fn c(hex: &str) -> u8 {
        let [r, g, b] = hex2rgb(hex);
        rgb_to_x256(r, g, b)
    }

    // ── hex2rgb tests ────────────────────────────────────────────────────

    #[test]
    fn test_hex2rgb_6digit() {
        assert_eq!(hex2rgb("#ff0000"), [255, 0, 0]);
        assert_eq!(hex2rgb("#00ff00"), [0, 255, 0]);
        assert_eq!(hex2rgb("#0000ff"), [0, 0, 255]);
        assert_eq!(hex2rgb("#ffffff"), [255, 255, 255]);
        assert_eq!(hex2rgb("#000000"), [0, 0, 0]);
        assert_eq!(hex2rgb("#1a2b3c"), [0x1a, 0x2b, 0x3c]);
    }

    #[test]
    fn test_hex2rgb_3digit() {
        assert_eq!(hex2rgb("#f00"), [255, 0, 0]);
        assert_eq!(hex2rgb("#0f0"), [0, 255, 0]);
        assert_eq!(hex2rgb("#00f"), [0, 0, 255]);
        assert_eq!(hex2rgb("#fff"), [255, 255, 255]);
        assert_eq!(hex2rgb("#000"), [0, 0, 0]);
        assert_eq!(hex2rgb("#abc"), [0xaa, 0xbb, 0xcc]);
    }

    #[test]
    fn test_hex2rgb_invalid() {
        assert_eq!(hex2rgb(""), [0, 0, 0]);
        assert_eq!(hex2rgb("#"), [0, 0, 0]);
        assert_eq!(hex2rgb("#gg0000"), [0, 0, 0]);
        assert_eq!(hex2rgb("#12345"), [0, 0, 0]);
        assert_eq!(hex2rgb("not-a-color"), [0, 0, 0]);
    }

    // ── rgb_to_x256 tests ────────────────────────────────────────────────

    #[test]
    fn test_rgb_to_x256_pure_red() {
        assert_eq!(rgb_to_x256(255, 0, 0), 196);
    }

    #[test]
    fn test_rgb_to_x256_white() {
        assert_eq!(rgb_to_x256(255, 255, 255), 231);
    }

    #[test]
    fn test_rgb_to_x256_black() {
        assert_eq!(rgb_to_x256(0, 0, 0), 16);
    }

    #[test]
    fn test_rgb_to_x256_mid_gray() {
        let idx = rgb_to_x256(128, 128, 128);
        assert!(
            (232..=255).contains(&idx),
            "Expected mid-gray to be in grayscale ramp 232..=255, got {}",
            idx
        );
    }

    // ── palette tests ────────────────────────────────────────────────────

    #[test]
    fn dark_palette_matches_hex() {
        let p = &DARK;
        assert_eq!(p.background, c("#000"));
        assert_eq!(p.water, c("#5f87ff"));
        assert_eq!(p.waterway, c("#a0c8f0"));
        assert_eq!(p.landuse_park, c("#7b5"));
        assert_eq!(p.landuse_wood, c("#6a4"));
        assert_eq!(p.landuse_cemetery, c("#e0e4dd"));
        assert_eq!(p.landuse_hospital, c("#d9b"));
        assert_eq!(p.landuse_school, c("#f0e8f8"));
        assert_eq!(p.landuse_overlay, c("#d8e8c8"));
        assert_eq!(p.building, c("#99b"));
        assert_eq!(p.aeroway, c("#f0ede9"));
        assert_eq!(p.road_motorway, c("#fc8"));
        assert_eq!(p.road_trunk_primary, c("#fea"));
        assert_eq!(p.road_street, c("#fff"));
        assert_eq!(p.road_path_pedestrian, c("#cba"));
        assert_eq!(p.road_rail, c("#bbb"));
        assert_eq!(p.tunnel_motorway, c("#ffdaa6"));
        assert_eq!(p.tunnel_link, c("#fff4c6"));
        assert_eq!(p.admin_level_2, c("#fff"));
        assert_eq!(p.admin_level_3, c("#aac"));
        assert_eq!(p.admin_maritime_2, c("#9bf"));
        assert_eq!(p.accent, c("#ff0"));
        assert_eq!(p.place_city, c("#f00"));
        assert_eq!(p.place_town, c("#d33"));
        assert_eq!(p.place_village, c("#c33"));
        assert_eq!(p.place_other, c("#b33"));
        assert_eq!(p.marine_label, c("#74aee9"));
        assert_eq!(p.water_label, c("#74aee9"));
        assert_eq!(p.poi_label_1, c("#ff0"));
        assert_eq!(p.poi_label_3, c("#cc0"));
        assert_eq!(p.poi_label_4, c("#aa0"));
        assert_eq!(p.rail_station_label, c("#666"));
        assert_eq!(p.road_label, c("#765"));
        assert_eq!(p.housenum_label, c("#88a"));
    }

    #[test]
    fn bright_palette_white_background() {
        let p = &BRIGHT;
        assert_eq!(p.background, 231); // white
        assert_eq!(p.fg, 16); // black
        // Verify key colors are dark enough for white background
        assert!(p.accent < 200, "accent should be dark on white bg");
        assert!(
            p.road_motorway < 200,
            "road_motorway should be dark on white bg"
        );
        assert!(p.place_city < 200, "place_city should be dark on white bg");
        assert!(
            p.admin_level_2 != 231,
            "admin borders should differ from white bg"
        );
    }

    #[test]
    fn xterm_to_rgb_cube_round_trips() {
        // Picks a known cube colour and verifies the RGB matches the
        // canonical 6-level encoding.
        let rgb = xterm_to_rgb(196); // pure red in cube
        assert_eq!(rgb, [255, 0, 0]);
        let rgb = xterm_to_rgb(46); // pure green
        assert_eq!(rgb, [0, 255, 0]);
        let rgb = xterm_to_rgb(21); // pure blue
        assert_eq!(rgb, [0, 0, 255]);
    }

    #[test]
    fn xterm_to_rgb_grayscale_ramp() {
        let rgb = xterm_to_rgb(232);
        assert_eq!(rgb, [8, 8, 8]);
        let rgb = xterm_to_rgb(255);
        let expected = (8u16 + 10 * 23) as u8;
        assert_eq!(rgb, [expected, expected, expected]);
    }

    #[test]
    fn dim_xterm_preserves_hue_for_pure_yellow() {
        // Yellow lives at cube (5, 5, 0) ≡ xterm 226. Stepping each
        // channel down by 2 lands at cube (3, 3, 0) ≡ xterm 142, which
        // remains R==G with B==0 — i.e. still yellow-ish, not olive
        // shifted toward green.
        let dim = dim_xterm(226);
        let [r, g, b] = xterm_to_rgb(dim);
        assert_eq!(r, g, "dim of yellow must keep R == G to remain yellow-hue");
        assert_eq!(b, 0, "dim of yellow must keep B == 0");
        assert!(r < 255, "dim of yellow must be strictly darker than 255");
    }

    #[test]
    fn dim_xterm_preserves_hue_for_pure_blue() {
        // Pure blue lives at cube (0, 0, 5) ≡ xterm 21. Stepping each
        // channel down by 2 lands at cube (0, 0, 3) ≡ xterm 19 — still
        // pure blue, just darker.
        let dim = dim_xterm(21);
        let [r, g, b] = xterm_to_rgb(dim);
        assert_eq!(r, 0, "dim of blue keeps R == 0");
        assert_eq!(g, 0, "dim of blue keeps G == 0");
        assert!(
            b < 255 && b > 0,
            "dim of blue stays in the blue channel, darker"
        );
    }

    #[test]
    fn dim_xterm_clamps_at_zero_for_already_dark_inputs() {
        // Black should stay black.
        assert_eq!(dim_xterm(16), 16);
        // A cube colour at level 1 in one channel and 0 in others
        // becomes (0, 0, 0) after a 2-step-down per channel.
        let dim = dim_xterm(17); // cube (0, 0, 1) — barely-blue
        assert_eq!(dim, 16, "dim of barely-blue clamps to black");
    }
}
