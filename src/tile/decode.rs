use std::collections::HashMap;
use std::io::Read;
use std::sync::Arc;

use prost::Message;
use rstar::{AABB, RTree, RTreeObject};

use crate::styler::StyleType;
use crate::styler::Styler;
use crate::styler::filter::PropertyValue;

pub mod proto {
    include!(concat!(env!("OUT_DIR"), "/vector_tile.rs"));
}

// ── Geometry types ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct Point {
    pub x: i32,
    pub y: i32,
}

// ── Feature ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct Feature {
    pub style_type: StyleType,
    pub label: Option<String>,
    pub sort: i64,
    pub points: Arc<Vec<Vec<Point>>>,
    pub color: u8,
    pub min_zoom: Option<f64>,
    pub max_zoom: Option<f64>,
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
fn decode_geometry(geometry: &[u32]) -> Vec<Vec<Point>> {
    let mut rings: Vec<Vec<Point>> = Vec::new();
    let mut current: Vec<Point> = Vec::new();
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
                    if !current.is_empty() {
                        rings.push(std::mem::take(&mut current));
                    }
                    let dx = zigzag(geometry[i]);
                    let dy = zigzag(geometry[i + 1]);
                    i += 2;
                    cx += dx;
                    cy += dy;
                    current.push(Point { x: cx, y: cy });
                }
            }
            2 => {
                // LineTo
                for _ in 0..count {
                    let dx = zigzag(geometry[i]);
                    let dy = zigzag(geometry[i + 1]);
                    i += 2;
                    cx += dx;
                    cy += dy;
                    current.push(Point { x: cx, y: cy });
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

fn decode_tags_into(
    tags: &[u32],
    keys: &[String],
    values: &[proto::tile::Value],
    props: &mut HashMap<String, PropertyValue>,
) {
    let mut j = 0;
    while j + 1 < tags.len() {
        let key_idx = tags[j] as usize;
        let val_idx = tags[j + 1] as usize;
        j += 2;

        let key = match keys.get(key_idx) {
            Some(k) => k,
            None => continue,
        };
        let proto_val = match values.get(val_idx) {
            Some(v) => v,
            None => continue,
        };

        if let Some(pv) = proto_value_to_pv(proto_val) {
            props.insert(key.clone(), pv);
        }
    }
}

fn proto_value_to_pv(v: &proto::tile::Value) -> Option<PropertyValue> {
    if let Some(s) = &v.string_value {
        return Some(PropertyValue::String(s.clone()));
    }
    if let Some(f) = v.float_value {
        return Some(PropertyValue::Number(f as f64));
    }
    if let Some(d) = v.double_value {
        return Some(PropertyValue::Number(d));
    }
    if let Some(i) = v.int_value {
        return Some(PropertyValue::Number(i as f64));
    }
    if let Some(u) = v.uint_value {
        return Some(PropertyValue::Number(u as f64));
    }
    if let Some(s) = v.sint_value {
        return Some(PropertyValue::Number(s as f64));
    }
    if let Some(b) = v.bool_value {
        return Some(PropertyValue::Bool(b));
    }
    None
}

// ── Label extractor ────────────────────────────────────────────────────────────

fn extract_label(props: &HashMap<String, PropertyValue>, language: &str) -> Option<String> {
    let lang_key = format!("name_{}", language);
    for key in &[lang_key.as_str(), "name_en", "name", "house_num"] {
        if let Some(PropertyValue::String(s)) = props.get(*key)
            && !s.is_empty()
        {
            return Some(s.clone());
        }
    }
    None
}

// ── Sort value extractor ───────────────────────────────────────────────────────

fn extract_sort(props: &HashMap<String, PropertyValue>) -> i64 {
    if let Some(v) = props.get("localrank").or_else(|| props.get("scalerank"))
        && let Some(n) = v.as_f64()
    {
        return n as i64;
    }
    0
}

// ── Bounds calculator ──────────────────────────────────────────────────────────

fn calculate_bounds(points: &[Vec<Point>]) -> (f64, f64, f64, f64) {
    let mut min_x = f64::MAX;
    let mut max_x = f64::MIN;
    let mut min_y = f64::MAX;
    let mut max_y = f64::MIN;

    for ring in points {
        for p in ring {
            let x = p.x as f64;
            let y = p.y as f64;
            if x < min_x {
                min_x = x;
            }
            if x > max_x {
                max_x = x;
            }
            if y < min_y {
                min_y = y;
            }
            if y > max_y {
                max_y = y;
            }
        }
    }

    if min_x == f64::MAX {
        (0.0, 0.0, 0.0, 0.0)
    } else {
        (min_x, max_x, min_y, max_y)
    }
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

pub fn decode(buffer: &[u8], styler: &Styler, language: &str) -> DecodedTile {
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
    // Reused across every feature in every layer to avoid per-feature
    // HashMap allocations. Cleared at the top of each iteration.
    let mut props: HashMap<String, PropertyValue> = HashMap::new();

    for layer in &tile.layers {
        let extent = layer.extent.unwrap_or(4096);
        let mut feats: Vec<Feature> = Vec::new();

        for feature in &layer.features {
            props.clear();
            decode_tags_into(&feature.tags, &layer.keys, &layer.values, &mut props);

            let type_str = match feature.r#type.unwrap_or(0) {
                1 => "Point",
                2 => "LineString",
                3 => "Polygon",
                _ => "Unknown",
            };
            props.insert(
                "$type".to_string(),
                PropertyValue::String(type_str.to_string()),
            );

            let style = match styler.get_style_for(&layer.name, &props) {
                Some(s) => s,
                None => continue,
            };

            let points = decode_geometry(&feature.geometry);
            if points.is_empty() {
                continue;
            }

            let label = if style.style_type == StyleType::Symbol {
                extract_label(&props, language)
            } else {
                None
            };
            let sort = extract_sort(&props);
            let (min_x, max_x, min_y, max_y) = calculate_bounds(&points);

            feats.push(Feature {
                style_type: style.style_type,
                label,
                sort,
                points: Arc::new(points),
                color: style.color,
                min_zoom: style.min_zoom,
                max_zoom: style.max_zoom,
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

    #[test]
    fn test_decompress_uncompressed() {
        let data = b"not gzipped data";
        let result = maybe_decompress(data);
        assert_eq!(result, data.to_vec());
    }

    #[test]
    fn test_bounds_calculation() {
        let points = vec![vec![
            Point { x: 10, y: 20 },
            Point { x: 30, y: 5 },
            Point { x: -5, y: 100 },
        ]];
        let (min_x, max_x, min_y, max_y) = calculate_bounds(&points);
        assert_eq!(min_x, -5.0);
        assert_eq!(max_x, 30.0);
        assert_eq!(min_y, 5.0);
        assert_eq!(max_y, 100.0);
    }
}
