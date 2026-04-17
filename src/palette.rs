//! Centralized color palette — every color in the app is defined here.
//!
//! Values are xterm-256 color indices (`u8`). This module has no dependencies
//! on ratatui or any UI crate, so it can be used by both the map renderer
//! and the UI layer.

/// All colors used by a single theme.
pub struct Palette {
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
    pub road_secondary_tertiary: u8,
    pub road_street: u8,
    pub road_service_track: u8,
    pub road_link: u8,
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

pub const DARK: Palette = Palette {
    background: 16,              // #000
    accent: 226,                 // Yellow (#ff0)
    accent_alt: 14,              // Cyan
    fg: 231,                     // White
    muted: 242,                  // DarkGray
    water: 69,                   // #5f87ff
    waterway: 153,               // #a0c8f0
    waterway_deep: 153,          // same as waterway in dark
    landuse_park: 107,           // #7b5
    landuse_wood: 71,            // #6a4
    landuse_cemetery: 254,       // #e0e4dd
    landuse_hospital: 175,       // #d9b
    landuse_school: 255,         // #f0e8f8
    landuse_overlay: 188,        // #d8e8c8
    building: 103,               // #99b
    aeroway: 255,                // #f0ede9
    road_motorway: 222,          // #fc8
    road_trunk_primary: 229,     // #fea
    road_secondary_tertiary: 229, // #fea
    road_street: 231,            // #fff
    road_service_track: 231,     // #fff
    road_link: 229,              // #fea
    road_path_pedestrian: 181,   // #cba
    road_rail: 250,              // #bbb
    road_casing_major: 222,      // same as motorway in dark
    road_casing_minor: 231,      // same as street in dark
    tunnel_motorway: 223,        // #ffdaa6
    tunnel_link: 230,            // #fff4c6
    admin_level_2: 231,          // #fff
    admin_level_3: 146,          // #aac
    admin_level_4: 146,          // same as level_3 in dark
    admin_disputed: 231,         // #fff
    admin_maritime_2: 111,       // #9bf
    admin_maritime_3: 111,       // #8af
    place_city: 196,             // #f00
    place_town: 167,             // #d33
    place_village: 167,          // #c33
    place_other: 131,            // #b33
    marine_label: 110,           // #74aee9
    water_label: 110,            // #74aee9
    poi_label_1: 226,            // #ff0
    poi_label_2: 226,            // #ee0
    poi_label_3: 184,            // #cc0
    poi_label_4: 142,            // #aa0
    rail_station_label: 241,     // #666
    airport_label: 241,          // #666
    road_label: 95,              // #765
    housenum_label: 103,         // #88a
};

/// Bright theme — white background, dark foreground colors.
pub const BRIGHT: Palette = Palette {
    background: 231,             // white
    accent: 25,                  // dark blue (#005faf)
    accent_alt: 30,              // teal (#008787)
    fg: 16,                      // black
    muted: 245,                  // mid gray
    water: 69,                   // #5f87ff (same as dark)
    waterway: 74,                // steel blue (#5fafd7)
    waterway_deep: 25,           // dark blue (#005faf)
    landuse_park: 71,            // green (#5faf5f)
    landuse_wood: 28,            // dark green (#008700)
    landuse_cemetery: 249,       // light gray (#b2b2b2)
    landuse_hospital: 217,       // light pink (#ffafaf)
    landuse_school: 183,         // light lavender (#d7afff)
    landuse_overlay: 151,        // sage (#afd7af)
    building: 249,               // light gray (#b2b2b2)
    aeroway: 250,                // silver (#bcbcbc)
    road_motorway: 166,          // dark orange (#d75f00)
    road_trunk_primary: 130,     // brown (#af5f00)
    road_secondary_tertiary: 137, // tan (#af875f)
    road_street: 245,            // mid gray (#8a8a8a)
    road_service_track: 248,     // light gray (#a8a8a8)
    road_link: 137,              // tan (#af875f)
    road_path_pedestrian: 138,   // rosy brown (#af8787)
    road_rail: 240,              // dark gray (#585858)
    road_casing_major: 166,      // dark orange (#d75f00)
    road_casing_minor: 245,      // mid gray
    tunnel_motorway: 180,        // muted orange (#d7af87)
    tunnel_link: 180,            // muted orange
    admin_level_2: 238,          // dark gray (#444444)
    admin_level_3: 243,          // mid gray (#767676)
    admin_level_4: 247,          // light gray (#9e9e9e)
    admin_disputed: 243,         // mid gray, dashed feel
    admin_maritime_2: 67,        // slate blue (#5f87af)
    admin_maritime_3: 103,       // dull blue (#8787af)
    place_city: 160,             // dark red (#d70000)
    place_town: 130,             // brown (#af5f00)
    place_village: 95,           // mauve (#875f5f)
    place_other: 242,            // gray (#6c6c6c)
    marine_label: 67,            // slate blue (#5f87af)
    water_label: 67,             // slate blue
    poi_label_1: 130,            // brown (#af5f00)
    poi_label_2: 137,            // tan (#af875f)
    poi_label_3: 101,            // olive (#87875f)
    poi_label_4: 243,            // gray (#767676)
    rail_station_label: 90,      // plum (#870087... actually #8700d7 area)
    airport_label: 25,           // dark blue (#005faf)
    road_label: 242,             // gray (#6c6c6c)
    housenum_label: 247,         // light gray (#9e9e9e)
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color;

    fn c(hex: &str) -> u8 {
        let [r, g, b] = color::hex2rgb(hex);
        color::rgb_to_x256(r, g, b)
    }

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
        assert_eq!(p.fg, 16);          // black
        // Verify key colors are dark enough for white background
        assert!(p.accent < 200, "accent should be dark on white bg");
        assert!(p.road_motorway < 200, "road_motorway should be dark on white bg");
        assert!(p.place_city < 200, "place_city should be dark on white bg");
        assert!(p.admin_level_2 != 231, "admin borders should differ from white bg");
    }
}
