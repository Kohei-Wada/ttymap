//! Polygon topology helpers used by the renderer's fill pass.
//!
//! Two pure functions, no state:
//!
//! - [`signed_area`] — surveyor's-formula area accumulated in `i64` so
//!   typical screen-pixel magnitudes can't overflow. Sign encodes
//!   winding: in MVT tile coords (Y-down) and screen coords (also
//!   Y-down), a clockwise ring has **positive** area and is the
//!   exterior; counter-clockwise (negative) is a hole.
//! - [`classify_polygon_groups`] — groups a flat ring list into
//!   `[outer, hole, hole, …]` ranges so a multi-polygon feature
//!   (one MVT POLYGON packing several disjoint outers, e.g. "all
//!   lakes in this tile") fills both regions instead of treating the
//!   second outer as a hole of the first (issue #101).
//!
//! Lifted out of `renderer.rs` so the math is independent of the
//! `Renderer` struct's borrow surface and can be unit-tested without
//! pulling a canvas through.

/// Surveyor's-formula signed area of a closed ring, accumulated in
/// `i64` to avoid overflow at typical screen-pixel magnitudes. In
/// MVT tile coords (Y-down) and screen coords (also Y-down), a CW
/// ring has positive signed area and is the **exterior**; CCW
/// (negative area) is a **hole**.
pub fn signed_area(ring: &[(i32, i32)]) -> i64 {
    if ring.len() < 3 {
        return 0;
    }
    let mut acc: i64 = 0;
    for i in 0..ring.len() {
        let (x0, y0) = ring[i];
        let (x1, y1) = ring[(i + 1) % ring.len()];
        acc += (x0 as i64) * (y1 as i64) - (x1 as i64) * (y0 as i64);
    }
    acc
}

/// Group a flat ring list into polygon groups. Each group starts at
/// an exterior ring (signed area > 0) and runs through any following
/// interior rings until the next exterior. Leading interior rings
/// (malformed input) are dropped. Returns half-open ranges into the
/// input slice.
pub fn classify_polygon_groups(rings: &[Vec<(i32, i32)>]) -> Vec<std::ops::Range<usize>> {
    let mut groups: Vec<std::ops::Range<usize>> = Vec::new();
    for (i, ring) in rings.iter().enumerate() {
        if signed_area(ring) > 0 {
            if let Some(last) = groups.last_mut() {
                last.end = i;
            }
            groups.push(i..i + 1);
        }
    }
    if let Some(last) = groups.last_mut() {
        last.end = rings.len();
    }
    groups
}

#[cfg(test)]
mod tests {
    use super::*;

    /// MVT spec: in tile (Y-down) coords, a CW ring has positive
    /// signed area and is the **exterior**; CCW (negative area) is a
    /// hole.
    #[test]
    fn signed_area_positive_for_clockwise_ring_in_y_down() {
        // Square traversed top-left → top-right → bottom-right →
        // bottom-left → close. CW visually in Y-down (screen) coords.
        let ring = vec![(0, 0), (10, 0), (10, 10), (0, 10)];
        assert!(
            signed_area(&ring) > 0,
            "CW ring in Y-down coords must report positive signed area"
        );
    }

    #[test]
    fn signed_area_negative_for_counterclockwise_ring_in_y_down() {
        // Same square, reversed → CCW in Y-down.
        let ring = vec![(0, 0), (0, 10), (10, 10), (10, 0)];
        assert!(
            signed_area(&ring) < 0,
            "CCW ring in Y-down coords must report negative signed area"
        );
    }

    #[test]
    fn signed_area_zero_for_degenerate_ring() {
        let ring = vec![(0, 0), (10, 0)]; // < 3 points
        assert_eq!(signed_area(&ring), 0);
    }

    /// Empty input → no groups.
    #[test]
    fn classify_polygon_groups_empty_input_yields_no_groups() {
        let rings: Vec<Vec<(i32, i32)>> = Vec::new();
        assert!(classify_polygon_groups(&rings).is_empty());
    }

    /// Single outer ring → one group spanning the whole slice.
    #[test]
    fn classify_polygon_groups_single_outer() {
        let rings = vec![vec![(0, 0), (10, 0), (10, 10), (0, 10)]];
        assert_eq!(classify_polygon_groups(&rings), vec![0..1]);
    }

    /// Outer + hole → one group [0..2].
    #[test]
    fn classify_polygon_groups_outer_with_hole() {
        let rings = vec![
            vec![(0, 0), (100, 0), (100, 100), (0, 100)], // CW = outer
            vec![(20, 20), (20, 40), (40, 40), (40, 20)], // CCW = hole
        ];
        assert_eq!(classify_polygon_groups(&rings), vec![0..2]);
    }

    /// Two disjoint outer rings (multi-polygon) → two separate
    /// groups. This is the case the pre-fix renderer mishandled — it
    /// treated the second outer as a hole of the first.
    #[test]
    fn classify_polygon_groups_two_outers_are_two_groups() {
        let rings = vec![
            vec![(0, 0), (10, 0), (10, 10), (0, 10)],
            vec![(50, 50), (60, 50), (60, 60), (50, 60)],
        ];
        assert_eq!(classify_polygon_groups(&rings), vec![0..1, 1..2]);
    }

    /// Mixed: outer, hole, outer, hole → two groups, each [outer,
    /// hole].
    #[test]
    fn classify_polygon_groups_outer_hole_outer_hole() {
        let rings = vec![
            vec![(0, 0), (100, 0), (100, 100), (0, 100)], // outer A
            vec![(20, 20), (20, 40), (40, 40), (40, 20)], // hole of A
            vec![(200, 0), (300, 0), (300, 100), (200, 100)], // outer B
            vec![(220, 20), (220, 40), (240, 40), (240, 20)], // hole of B
        ];
        assert_eq!(classify_polygon_groups(&rings), vec![0..2, 2..4]);
    }

    /// Leading hole (malformed input) is skipped; valid outer behind
    /// it still becomes a group.
    #[test]
    fn classify_polygon_groups_drops_leading_holes() {
        let rings = vec![
            vec![(0, 0), (0, 10), (10, 10), (10, 0)], // CCW: hole, no parent
            vec![(50, 50), (60, 50), (60, 60), (50, 60)], // CW: outer
        ];
        assert_eq!(classify_polygon_groups(&rings), vec![1..2]);
    }
}
