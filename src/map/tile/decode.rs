use std::collections::HashMap;
use std::io::Read;
use std::sync::Arc;

use prost::Message;
use rstar::{AABB, RTree, RTreeObject};

use super::property::PropertyValue;

pub mod proto {
    include!(concat!(env!("OUT_DIR"), "/vector_tile.rs"));
}

// ── Geometry types ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
/// A point in the MVT **tile-local** coordinate space.
///
/// Coordinates are integers in the layer's `extent` (typically 0..4096).
/// They are distinct from *screen pixels* and from *world Mercator*
/// coordinates (see `geo::TileCoord`); converting between them happens
/// at render time in [`crate::map::render::renderer`].
pub struct TilePoint {
    pub x: i32,
    pub y: i32,
}

// ── Feature ────────────────────────────────────────────────────────────────────
//
// Carries raw MVT data only. Style resolution (color, style_type, min/max_zoom)
// and label extraction live in the render layer; this keeps tile decode a pure
// protobuf → geometry transform with no UI-layer dependencies.

#[derive(Debug, Clone)]
pub struct Feature {
    pub layer_name: Arc<str>,
    pub properties: Arc<HashMap<Arc<str>, PropertyValue>>,
    pub points: Arc<Vec<Vec<TilePoint>>>,
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

// ── Tile structures ────────────────────────────────────────────────────────────

pub struct TileLayer {
    pub extent: u32,
    pub tree: RTree<Feature>,
}

pub struct DecodedTile {
    pub layers: HashMap<String, TileLayer>,
}

// ── Zigzag decode ──────────────────────────────────────────────────────────────

#[inline]
fn zigzag(n: u32) -> i32 {
    ((n >> 1) as i32) ^ -((n & 1) as i32)
}

// ── Geometry decoder ───────────────────────────────────────────────────────────

/// Decode MVT geometry into rings of Points.
/// For POLYGON (geom_type 3), all rings (outer + holes) are flattened into a
/// single ring list.  For other types each ring is separate.
fn decode_geometry(geometry: &[u32]) -> Vec<Vec<TilePoint>> {
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

// ── Tag decoder ────────────────────────────────────────────────────────────────

/// Tag decoder. `keys_arc` and `values_pool` are pre-computed once per layer
/// so each feature only pays for `Arc::clone` (refcount bump) rather than a
/// fresh heap allocation per tag.
fn decode_tags_into(
    tags: &[u32],
    keys_arc: &[Arc<str>],
    values_pool: &[Option<PropertyValue>],
    props: &mut HashMap<Arc<str>, PropertyValue>,
) {
    let mut j = 0;
    while j + 1 < tags.len() {
        let key_idx = tags[j] as usize;
        let val_idx = tags[j + 1] as usize;
        j += 2;

        let key = match keys_arc.get(key_idx) {
            Some(k) => k,
            None => continue,
        };
        let Some(Some(pv)) = values_pool.get(val_idx) else {
            continue;
        };
        props.insert(key.clone(), pv.clone());
    }
}

fn proto_value_to_pv(v: &proto::tile::Value) -> Option<PropertyValue> {
    if let Some(s) = &v.string_value {
        return Some(PropertyValue::String(Arc::from(s.as_str())));
    }
    if let Some(b) = v.bool_value {
        return Some(PropertyValue::Bool(b));
    }
    // All numeric proto types collapse to Number(f64).
    let num = v
        .float_value
        .map(|n| n as f64)
        .or(v.double_value)
        .or(v.int_value.map(|n| n as f64))
        .or(v.uint_value.map(|n| n as f64))
        .or(v.sint_value.map(|n| n as f64))?;
    Some(PropertyValue::Number(num))
}

// ── Bounds calculator ──────────────────────────────────────────────────────────

fn calculate_bounds(points: &[Vec<TilePoint>]) -> (f64, f64, f64, f64) {
    points
        .iter()
        .flatten()
        .fold(None, |acc: Option<(f64, f64, f64, f64)>, p| {
            let x = p.x as f64;
            let y = p.y as f64;
            Some(match acc {
                None => (x, x, y, y),
                Some((min_x, max_x, min_y, max_y)) => {
                    (min_x.min(x), max_x.max(x), min_y.min(y), max_y.max(y))
                }
            })
        })
        .unwrap_or((0.0, 0.0, 0.0, 0.0))
}

// ── Decompression ──────────────────────────────────────────────────────────────

fn maybe_decompress(buffer: &[u8]) -> Vec<u8> {
    if buffer.len() >= 2 && buffer[0] == 0x1f && buffer[1] == 0x8b {
        let mut decoder = flate2::read::GzDecoder::new(buffer);
        let mut out = Vec::new();
        decoder.read_to_end(&mut out).unwrap_or(0);
        out
    } else {
        buffer.to_vec()
    }
}

// ── Main decode ────────────────────────────────────────────────────────────────

/// Pure MVT decode: parse protobuf and build an R-tree per layer. Does not
/// consult any styler — styling, min/max-zoom filtering, and label extraction
/// all happen at render time.
pub fn decode(buffer: &[u8]) -> DecodedTile {
    let data = maybe_decompress(buffer);

    let tile = match proto::Tile::decode(data.as_slice()) {
        Ok(t) => t,
        Err(_) => {
            return DecodedTile {
                layers: HashMap::new(),
            };
        }
    };

    let mut decoded_layers: HashMap<String, TileLayer> = HashMap::new();

    // "$type" is inserted on every feature but its content is one of four
    // string constants; intern them once here so each feature's insert is a
    // refcount bump. Same for the key itself.
    let type_key: Arc<str> = Arc::from("$type");
    let type_point: Arc<str> = Arc::from("Point");
    let type_linestring: Arc<str> = Arc::from("LineString");
    let type_polygon: Arc<str> = Arc::from("Polygon");
    let type_unknown: Arc<str> = Arc::from("Unknown");

    for layer in &tile.layers {
        let extent = layer.extent.unwrap_or(4096);
        let layer_name: Arc<str> = Arc::from(layer.name.as_str());

        // Pre-wrap the layer's key pool and value pool exactly once. Every
        // feature in this layer references the same `Arc<str>` instances by
        // cheap clone rather than allocating fresh Strings per tag.
        let keys_arc: Vec<Arc<str>> = layer.keys.iter().map(|k| Arc::from(k.as_str())).collect();
        let values_pool: Vec<Option<PropertyValue>> =
            layer.values.iter().map(proto_value_to_pv).collect();

        let mut feats: Vec<Feature> = Vec::new();

        for feature in &layer.features {
            let mut props: HashMap<Arc<str>, PropertyValue> = HashMap::new();
            decode_tags_into(&feature.tags, &keys_arc, &values_pool, &mut props);

            let type_str = match feature.r#type.unwrap_or(0) {
                1 => type_point.clone(),
                2 => type_linestring.clone(),
                3 => type_polygon.clone(),
                _ => type_unknown.clone(),
            };
            props.insert(type_key.clone(), PropertyValue::String(type_str));

            let points = decode_geometry(&feature.geometry);
            if points.is_empty() {
                continue;
            }
            let (min_x, max_x, min_y, max_y) = calculate_bounds(&points);

            feats.push(Feature {
                layer_name: layer_name.clone(),
                properties: Arc::new(props),
                points: Arc::new(points),
                min_x,
                max_x,
                min_y,
                max_y,
            });
        }

        if !feats.is_empty() {
            let tree = RTree::bulk_load(feats);
            decoded_layers.insert(layer.name.clone(), TileLayer { extent, tree });
        }
    }

    DecodedTile {
        layers: decoded_layers,
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────────

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

    #[test]
    fn test_decompress_uncompressed() {
        let data = b"not gzipped data";
        let result = maybe_decompress(data);
        assert_eq!(result, data.to_vec());
    }

    #[test]
    fn test_bounds_calculation() {
        let points = vec![vec![
            TilePoint { x: 10, y: 20 },
            TilePoint { x: 30, y: 5 },
            TilePoint { x: -5, y: 100 },
        ]];
        let (min_x, max_x, min_y, max_y) = calculate_bounds(&points);
        assert_eq!(min_x, -5.0);
        assert_eq!(max_x, 30.0);
        assert_eq!(min_y, 5.0);
        assert_eq!(max_y, 100.0);
    }
}
