//! MVT feature property value type and helpers for reading it.
//!
//! The value enum is the Rust-side image of the Mapbox Vector Tile
//! `tile.Value` message. It lives here (not under `styler/`) because
//! it's produced by decode and consumed by every downstream layer —
//! styler for filter matching, renderer for label / sort extraction.
//! Keeping it in `tile/` keeps module dependencies a DAG (styler →
//! tile, never the reverse).

use std::collections::HashMap;
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq)]
pub enum PropertyValue {
    /// String values use `Arc<str>` so tiles can intern a layer's key
    /// and value pool once and hand out cheap refcount clones to each
    /// feature instead of paying a per-tag heap allocation.
    String(Arc<str>),
    Number(f64),
    Bool(bool),
}

impl PropertyValue {
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            PropertyValue::Number(n) => Some(*n),
            _ => None,
        }
    }
}

/// Pick a display label from a feature's properties using the requested
/// language (falling back to `name_en`, then `name`, then `house_num`).
pub fn extract_label(props: &HashMap<Arc<str>, PropertyValue>, language: &str) -> Option<String> {
    let lang_key = format!("name_{}", language);
    for key in &[lang_key.as_str(), "name_en", "name", "house_num"] {
        if let Some(PropertyValue::String(s)) = props.get(*key)
            && !s.is_empty()
        {
            return Some(s.to_string());
        }
    }
    None
}

/// Sort key used to order labels (smaller = drawn first, so higher priority).
pub fn extract_sort(props: &HashMap<Arc<str>, PropertyValue>) -> i64 {
    if let Some(v) = props.get("localrank").or_else(|| props.get("scalerank"))
        && let Some(n) = v.as_f64()
    {
        return n as i64;
    }
    0
}
