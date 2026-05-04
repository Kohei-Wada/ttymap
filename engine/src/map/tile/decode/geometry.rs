//! MVT geometry-stream decoder.
//!
//! Per-feature `geometry` is a sequence of zigzag-encoded varints with
//! a tiny command vocabulary (MoveTo / LineTo / ClosePath). This
//! module turns that stream into rings of `TilePoint`s. Pure function;
//! no allocation outside the returned `Vec`.

use super::TilePoint;

#[inline]
pub(super) fn zigzag(n: u32) -> i32 {
    ((n >> 1) as i32) ^ -((n & 1) as i32)
}

/// Decode an MVT geometry stream into rings of points.
///
/// For POLYGON (geom_type 3), all rings (outer + holes) are flattened
/// into a single ring list — winding-based classification happens at
/// render time. For other geom_types each ring is separate.
///
/// Robust against truncated / malformed streams: bounds-checks every
/// parameter pair (issue #102) so a header that claims more points
/// than the buffer carries leaves a partial result instead of
/// panicking.
pub(super) fn decode_geometry(geometry: &[u32]) -> Vec<Vec<TilePoint>> {
    let mut rings: Vec<Vec<TilePoint>> = Vec::new();
    let mut current: Vec<TilePoint> = Vec::new();
    let mut cx: i32 = 0;
    let mut cy: i32 = 0;

    let mut i = 0;
    while i < geometry.len() {
        let cmd_int = geometry[i];
        i += 1;

        let cmd = cmd_int & 0x7;
        let count = (cmd_int >> 3) as usize;

        match cmd {
            1 => {
                // MoveTo
                for _ in 0..count {
                    // Truncated / malformed tile: the command header
                    // claims more parameter pairs than the buffer
                    // actually carries. Stop instead of panicking on
                    // OOB; partial geometry is acceptable. See #102.
                    if i + 1 >= geometry.len() {
                        break;
                    }
                    if !current.is_empty() {
                        rings.push(std::mem::take(&mut current));
                    }
                    let dx = zigzag(geometry[i]);
                    let dy = zigzag(geometry[i + 1]);
                    i += 2;
                    cx += dx;
                    cy += dy;
                    current.push(TilePoint { x: cx, y: cy });
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
                    current.push(TilePoint { x: cx, y: cy });
                }
            }
            7 => {
                // ClosePath — append copy of first point to close the ring
                if let Some(first) = current.first().cloned() {
                    current.push(first);
                }
                rings.push(std::mem::take(&mut current));
            }
            _ => {}
        }
    }

    if !current.is_empty() {
        rings.push(current);
    }

    rings
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
        // MoveTo(count=1), x=20 (zigzag -> 10), y=40 (zigzag -> 20)
        // Command: (1 << 3) | 1 = 9
        let geometry = vec![9u32, 20, 40];
        let rings = decode_geometry(&geometry);
        assert_eq!(rings.len(), 1);
        assert_eq!(rings[0].len(), 1);
        assert_eq!(rings[0][0].x, 10);
        assert_eq!(rings[0][0].y, 20);
    }

    #[test]
    fn test_decode_geometry_line() {
        // MoveTo(count=1), x=0(0), y=0(0)
        // LineTo(count=1), dx=2(1), dy=4(2)
        // MoveTo cmd: (1 << 3) | 1 = 9
        // LineTo cmd: (1 << 3) | 2 = 10
        let geometry = vec![9u32, 0, 0, 10, 2, 4];
        let rings = decode_geometry(&geometry);
        assert_eq!(rings.len(), 1);
        assert_eq!(rings[0].len(), 2);
        assert_eq!(rings[0][0].x, 0);
        assert_eq!(rings[0][0].y, 0);
        assert_eq!(rings[0][1].x, 1);
        assert_eq!(rings[0][1].y, 2);
    }

    /// Regression for issue #102. A MoveTo command claiming more
    /// points than the geometry buffer actually carries (truncated
    /// fetch, server bug, adversarial input) must NOT panic the
    /// decoder. The valid prefix should still come back.
    #[test]
    fn decode_geometry_truncated_moveto_does_not_panic() {
        // MoveTo(count=3) but only 2 (dx,dy) pairs follow.
        // 1 | (3 << 3) = 25.
        let geometry = vec![25u32, 2, 2, 2, 2];
        let rings = decode_geometry(&geometry);
        // First two MoveTo iterations succeed: each starts a fresh ring
        // with a single point. Third would index past the end → break.
        assert_eq!(rings.len(), 2);
        assert_eq!(rings[0].len(), 1);
        assert_eq!((rings[0][0].x, rings[0][0].y), (1, 1));
        assert_eq!(rings[1].len(), 1);
        assert_eq!((rings[1][0].x, rings[1][0].y), (2, 2));
    }

    /// Same regression on the LineTo branch.
    #[test]
    fn decode_geometry_truncated_lineto_does_not_panic() {
        // MoveTo(1) → (1,1). LineTo(count=3) but only 1 pair follows.
        // MoveTo: 1 | (1<<3) = 9. LineTo: 2 | (3<<3) = 26.
        let geometry = vec![9u32, 2, 2, 26, 2, 2];
        let rings = decode_geometry(&geometry);
        // MoveTo writes (1,1). LineTo iter 1 writes (2,2). Iter 2 of
        // LineTo would index past the end → break. Single ring with
        // both points.
        assert_eq!(rings.len(), 1);
        assert_eq!(rings[0].len(), 2);
        assert_eq!((rings[0][0].x, rings[0][0].y), (1, 1));
        assert_eq!((rings[0][1].x, rings[0][1].y), (2, 2));
    }
}
