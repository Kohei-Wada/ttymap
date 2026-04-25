//! MVT (Mapbox Vector Tile) decoder.
//!
//! Pure: takes raw bytes (optionally gzipped), returns a `DecodedTile`
//! with per-layer R-trees of `Feature`s. No styling, no zoom filter,
//! no label extraction — those happen at render time so the cache
//! does not need flushing on theme / language changes.
//!
//! Sub-modules:
//! - `geometry` — zigzag + the MVT command-stream geometry decoder
//! - `tags`     — tag-pair decoder + protobuf value → `PropertyValue`
//! - `decompress` — gzip sniff + transparent decompression

use std::collections::HashMap;
use std::sync::Arc;

use prost::Message;
use rstar::{AABB, RTree, RTreeObject};

use super::property::PropertyValue;

mod decompress;
mod geometry;
mod tags;

pub mod proto {
    include!(concat!(env!("OUT_DIR"), "/vector_tile.rs"));
}

// ── Geometry types ─────────────────────────────────────────────────────────────

/// A point in the MVT **tile-local** coordinate space.
///
/// Coordinates are integers in the layer's `extent` (typically 0..4096).
/// They are distinct from *screen pixels* and from *world Mercator*
/// coordinates (see `geo::TileCoord`); converting between them happens
/// at render time in [`crate::map::render::renderer`].
#[derive(Debug, Clone)]
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

// ── Main decode ────────────────────────────────────────────────────────────────

/// Pure MVT decode: parse protobuf and build an R-tree per layer. Does not
/// consult any styler — styling, min/max-zoom filtering, and label extraction
/// all happen at render time.
pub fn decode(buffer: &[u8]) -> DecodedTile {
    let data = decompress::maybe_decompress(buffer);

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
            layer.values.iter().map(tags::proto_value_to_pv).collect();

        let mut feats: Vec<Feature> = Vec::new();

        for feature in &layer.features {
            let mut props: HashMap<Arc<str>, PropertyValue> = HashMap::new();
            tags::decode_tags_into(&feature.tags, &keys_arc, &values_pool, &mut props);

            let type_str = match feature.r#type.unwrap_or(0) {
                1 => type_point.clone(),
                2 => type_linestring.clone(),
                3 => type_polygon.clone(),
                _ => type_unknown.clone(),
            };
            props.insert(type_key.clone(), PropertyValue::String(type_str));

            let points = geometry::decode_geometry(&feature.geometry);
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

    #[test]
    fn calculate_bounds_empty_input_yields_zero_box() {
        let (min_x, max_x, min_y, max_y) = calculate_bounds(&[]);
        assert_eq!((min_x, max_x, min_y, max_y), (0.0, 0.0, 0.0, 0.0));
    }

    #[test]
    fn calculate_bounds_single_point() {
        let pts = vec![vec![TilePoint { x: 7, y: -3 }]];
        assert_eq!(calculate_bounds(&pts), (7.0, 7.0, -3.0, -3.0));
    }

    #[test]
    fn calculate_bounds_walks_all_rings() {
        // Bounds must be over the union of all rings, not just the first.
        let pts = vec![
            vec![TilePoint { x: 0, y: 0 }, TilePoint { x: 10, y: 10 }],
            vec![TilePoint { x: -5, y: 50 }],
        ];
        assert_eq!(calculate_bounds(&pts), (-5.0, 10.0, 0.0, 50.0));
    }

    /// `decode` must never panic on malformed top-level input — protobuf
    /// parse failure returns an empty `DecodedTile`. Useful as a smoke
    /// guard in conjunction with #102 (geometry truncation handled per
    /// feature).
    #[test]
    fn decode_returns_empty_on_garbage_protobuf() {
        let result = decode(b"not a tile");
        assert!(result.layers.is_empty());
    }

    #[test]
    fn decode_returns_empty_on_empty_input() {
        let result = decode(b"");
        assert!(result.layers.is_empty());
    }
}
