//! MVT tag decoder. Each feature carries `tags: Vec<u32>` which is
//! a flat sequence of `(key_index, value_index)` pairs; both indices
//! point into per-layer string / value pools. We pre-wrap the pools
//! once per layer (in `super::decode`) so each feature only pays for
//! `Arc::clone` rather than a fresh heap allocation per tag.

use std::collections::HashMap;
use std::sync::Arc;

use super::proto;
use crate::core::map::tile::property::PropertyValue;

pub(super) fn decode_tags_into(
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

/// Collapse a protobuf `Value` into our internal `PropertyValue`.
/// All numeric proto types (float/double/int/uint/sint) collapse to
/// `Number(f64)` — the renderer doesn't care about precision class.
pub(super) fn proto_value_to_pv(v: &proto::tile::Value) -> Option<PropertyValue> {
    if let Some(s) = &v.string_value {
        return Some(PropertyValue::String(Arc::from(s.as_str())));
    }
    if let Some(b) = v.bool_value {
        return Some(PropertyValue::Bool(b));
    }
    let num = v
        .float_value
        .map(|n| n as f64)
        .or(v.double_value)
        .or(v.int_value.map(|n| n as f64))
        .or(v.uint_value.map(|n| n as f64))
        .or(v.sint_value.map(|n| n as f64))?;
    Some(PropertyValue::Number(num))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys() -> Vec<Arc<str>> {
        vec![Arc::from("class"), Arc::from("name"), Arc::from("ele")]
    }

    fn values() -> Vec<Option<PropertyValue>> {
        vec![
            Some(PropertyValue::String(Arc::from("park"))),
            Some(PropertyValue::Number(42.0)),
            Some(PropertyValue::Bool(true)),
        ]
    }

    #[test]
    fn decodes_paired_indices_into_props() {
        // class=park, name=42 (numeric), ele=true
        let tags = vec![0u32, 0, 1, 1, 2, 2];
        let mut props = HashMap::new();
        decode_tags_into(&tags, &keys(), &values(), &mut props);
        assert_eq!(props.len(), 3);
        assert!(
            matches!(props.get("class"), Some(PropertyValue::String(s)) if s.as_ref() == "park")
        );
        assert!(matches!(props.get("name"), Some(PropertyValue::Number(n)) if *n == 42.0));
        assert!(matches!(props.get("ele"), Some(PropertyValue::Bool(true))));
    }

    #[test]
    fn skips_pair_when_key_index_out_of_range() {
        // key=99 invalid, key=0 valid → only the second pair lands.
        let tags = vec![99u32, 0, 0, 1];
        let mut props = HashMap::new();
        decode_tags_into(&tags, &keys(), &values(), &mut props);
        assert_eq!(props.len(), 1);
        assert!(props.contains_key("class"));
    }

    #[test]
    fn skips_pair_when_value_index_out_of_range() {
        let tags = vec![0u32, 99];
        let mut props = HashMap::new();
        decode_tags_into(&tags, &keys(), &values(), &mut props);
        assert!(props.is_empty());
    }

    #[test]
    fn skips_pair_when_value_is_none_in_pool() {
        // Value pool has a `None` slot — proto value couldn't be decoded
        // (e.g. all-zero proto::tile::Value). Tag pointing at it is skipped.
        let mut vals = values();
        vals[1] = None;
        let tags = vec![1u32, 1];
        let mut props = HashMap::new();
        decode_tags_into(&tags, &keys(), &vals, &mut props);
        assert!(props.is_empty());
    }

    #[test]
    fn ignores_dangling_odd_final_index() {
        // Odd-length tag stream: the final unpaired index is ignored.
        let tags = vec![0u32, 0, 1];
        let mut props = HashMap::new();
        decode_tags_into(&tags, &keys(), &values(), &mut props);
        assert_eq!(props.len(), 1);
        assert!(props.contains_key("class"));
    }

    #[test]
    fn proto_value_string_takes_precedence() {
        let v = proto::tile::Value {
            string_value: Some("hello".to_string()),
            int_value: Some(7),
            ..Default::default()
        };
        assert!(
            matches!(proto_value_to_pv(&v), Some(PropertyValue::String(s)) if s.as_ref() == "hello")
        );
    }

    #[test]
    fn proto_value_bool_takes_precedence_over_numeric() {
        let v = proto::tile::Value {
            bool_value: Some(true),
            int_value: Some(7),
            ..Default::default()
        };
        assert!(matches!(
            proto_value_to_pv(&v),
            Some(PropertyValue::Bool(true))
        ));
    }

    #[test]
    fn proto_value_numeric_collapses_to_f64() {
        let v = proto::tile::Value {
            int_value: Some(-3),
            ..Default::default()
        };
        assert!(matches!(proto_value_to_pv(&v), Some(PropertyValue::Number(n)) if n == -3.0));

        let v = proto::tile::Value {
            float_value: Some(1.5),
            ..Default::default()
        };
        assert!(
            matches!(proto_value_to_pv(&v), Some(PropertyValue::Number(n)) if (n - 1.5).abs() < 1e-6)
        );

        let v = proto::tile::Value {
            uint_value: Some(42),
            ..Default::default()
        };
        assert!(matches!(proto_value_to_pv(&v), Some(PropertyValue::Number(n)) if n == 42.0));
    }

    #[test]
    fn proto_value_empty_yields_none() {
        let v = proto::tile::Value::default();
        assert!(proto_value_to_pv(&v).is_none());
    }
}
