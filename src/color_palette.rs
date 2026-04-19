//! Centralized color palette — every color in the app is defined here.
//!
//! Values are xterm-256 color indices (`u8`). This module has no dependencies
//! on ratatui or any UI crate, so it can be used by both the map renderer
//! and the UI layer.
//!
//! [`ThemeId`] is the single source of truth for "which theme is active":
//! pick one from the config, then derive everything else from it — the
//! [`ColorPalette`] the UI reads, the `styler::Styler` the map renderer reads,
//! and the display name shown to the user.

/// Identifies which theme the app is running with. Derives the concrete
/// [`ColorPalette`] and, separately, the set of styling rules consumed by
/// `styler::Styler`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ThemeId {
    #[default]
    Dark,
    Bright,
}

impl ThemeId {
    /// Parse a config string. Unknown names fall back to [`ThemeId::Dark`].
    pub fn from_name(name: &str) -> Self {
        match name {
            "bright" => Self::Bright,
            _ => Self::Dark,
        }
    }

    /// The palette this theme ships with.
    pub fn palette(self) -> &'static ColorPalette {
        match self {
            Self::Dark => &DARK,
            Self::Bright => &BRIGHT,
        }
    }

    /// Canonical lowercase name used for logging / `styler.name()`.
    pub fn name(self) -> &'static str {
        match self {
            Self::Dark => "dark",
            Self::Bright => "bright",
        }
    }

    /// Every known theme, in the order they should appear in UI
    /// listings (command palette, help overlay). Extend here when
    /// adding a new preset; the rest of the app discovers them through
    /// this single table.
    pub fn all() -> &'static [ThemeId] {
        &[ThemeId::Dark, ThemeId::Bright]
    }
}

/// All colors used by a single theme.
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
    pub road_casing_major: u8,
    pub road_casing_minor: u8,
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
    road_casing_major: 222,    // same as motorway in dark
    road_casing_minor: 231,    // same as street in dark
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
    road_trunk_primary: 130,   // brown (#af5f00)
    road_street: 245,          // mid gray (#8a8a8a)
    road_path_pedestrian: 138, // rosy brown (#af8787)
    road_rail: 240,            // dark gray (#585858)
    road_casing_major: 166,    // dark orange (#d75f00)
    road_casing_minor: 245,    // mid gray
    tunnel_motorway: 180,      // muted orange (#d7af87)
    tunnel_link: 180,          // muted orange
    admin_level_2: 238,        // dark gray (#444444)
    admin_level_3: 243,        // mid gray (#767676)
    admin_level_4: 247,        // light gray (#9e9e9e)
    admin_disputed: 243,       // mid gray, dashed feel
    admin_maritime_2: 67,      // slate blue (#5f87af)
    admin_maritime_3: 103,     // dull blue (#8787af)
    place_city: 160,           // dark red (#d70000)
    place_town: 130,           // brown (#af5f00)
    place_village: 95,         // mauve (#875f5f)
    place_other: 242,          // gray (#6c6c6c)
    marine_label: 67,          // slate blue (#5f87af)
    water_label: 67,           // slate blue
    poi_label_1: 130,          // brown (#af5f00)
    poi_label_2: 137,          // tan (#af875f)
    poi_label_3: 101,          // olive (#87875f)
    poi_label_4: 243,          // gray (#767676)
    rail_station_label: 90,    // plum (#870087... actually #8700d7 area)
    airport_label: 25,         // dark blue (#005faf)
    road_label: 242,           // gray (#6c6c6c)
    housenum_label: 247,       // light gray (#9e9e9e)
};

#[cfg(test)]
mod tests {
    use super::*;

    // ── Color conversion helpers ─────────────────────────────────────────
    //
    // These exist solely to verify that the hard-coded xterm-256 palette
    // indices above match their intended hex colors. They were previously
    // in a top-level `color` module but had no non-test callers, so they
    // live here now as test-only helpers.

    /// Parse a hex color string (`#RGB` or `#RRGGBB`) into an RGB array.
    /// Returns `[0, 0, 0]` for invalid input.
    fn hex2rgb(color: &str) -> [u8; 3] {
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
    fn rgb_to_x256(r: u8, g: u8, b: u8) -> u8 {
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
}
