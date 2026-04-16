/// Parse a hex color string (#RGB or #RRGGBB) into an RGB array.
/// Returns [0, 0, 0] for invalid input.
pub fn hex2rgb(color: &str) -> [u8; 3] {
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
/// Considers the 6x6x6 color cube (indices 16..=231) and grayscale ramp (232..=255).
pub fn rgb_to_x256(r: u8, g: u8, b: u8) -> u8 {
    // 6x6x6 color cube levels: 0, 95, 135, 175, 215, 255
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

    // Grayscale ramp: indices 232..=255, values 8, 18, 28, ..., 238
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

#[cfg(test)]
mod tests {
    use super::*;

    // --- hex2rgb tests ---

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
        // #f00 -> each digit expanded: f -> ff = 255
        assert_eq!(hex2rgb("#f00"), [255, 0, 0]);
        assert_eq!(hex2rgb("#0f0"), [0, 255, 0]);
        assert_eq!(hex2rgb("#00f"), [0, 0, 255]);
        assert_eq!(hex2rgb("#fff"), [255, 255, 255]);
        assert_eq!(hex2rgb("#000"), [0, 0, 0]);
        // #abc -> a=170, b=187, c=204
        assert_eq!(hex2rgb("#abc"), [0xaa, 0xbb, 0xcc]);
    }

    #[test]
    fn test_hex2rgb_invalid() {
        assert_eq!(hex2rgb(""), [0, 0, 0]);
        assert_eq!(hex2rgb("#"), [0, 0, 0]);
        assert_eq!(hex2rgb("#gg0000"), [0, 0, 0]);
        assert_eq!(hex2rgb("#12345"), [0, 0, 0]); // wrong length
        assert_eq!(hex2rgb("not-a-color"), [0, 0, 0]);
    }

    // --- rgb_to_x256 tests ---

    #[test]
    fn test_rgb_to_x256_pure_red() {
        // Pure red (255, 0, 0) should map to index 196
        assert_eq!(rgb_to_x256(255, 0, 0), 196);
    }

    #[test]
    fn test_rgb_to_x256_white() {
        // White (255, 255, 255) should map to index 231
        assert_eq!(rgb_to_x256(255, 255, 255), 231);
    }

    #[test]
    fn test_rgb_to_x256_black() {
        // Black (0, 0, 0) should map to index 16
        assert_eq!(rgb_to_x256(0, 0, 0), 16);
    }

    #[test]
    fn test_rgb_to_x256_mid_gray() {
        // Mid-gray should map to the grayscale ramp (232..=255)
        let idx = rgb_to_x256(128, 128, 128);
        assert!(
            (232..=255).contains(&idx),
            "Expected mid-gray to be in grayscale ramp 232..=255, got {}",
            idx
        );
    }
}
