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

#[derive(Debug, Clone)]
pub enum Filter {
    Always,
    Eq(String, PropertyValue),
    NotEq(String, PropertyValue),
    In(String, Vec<PropertyValue>),
    NotIn(String, Vec<PropertyValue>),
    Has(String),
    NotHas(String),
    Gt(String, f64),
    Gte(String, f64),
    Lt(String, f64),
    Lte(String, f64),
    All(Vec<Filter>),
    Any(Vec<Filter>),
    None(Vec<Filter>),
}

impl Filter {
    pub fn eval(&self, props: &HashMap<Arc<str>, PropertyValue>) -> bool {
        match self {
            Filter::Always => true,
            Filter::Eq(key, val) => props.get(key.as_str()) == Some(val),
            Filter::NotEq(key, val) => props.get(key.as_str()) != Some(val),
            Filter::In(key, vals) => props.get(key.as_str()).is_some_and(|v| vals.contains(v)),
            Filter::NotIn(key, vals) => props.get(key.as_str()).is_none_or(|v| !vals.contains(v)),
            Filter::Has(key) => props.contains_key(key.as_str()),
            Filter::NotHas(key) => !props.contains_key(key.as_str()),
            Filter::Gt(key, val) => props
                .get(key.as_str())
                .and_then(|v| v.as_f64())
                .is_some_and(|n| n > *val),
            Filter::Gte(key, val) => props
                .get(key.as_str())
                .and_then(|v| v.as_f64())
                .is_some_and(|n| n >= *val),
            Filter::Lt(key, val) => props
                .get(key.as_str())
                .and_then(|v| v.as_f64())
                .is_some_and(|n| n < *val),
            Filter::Lte(key, val) => props
                .get(key.as_str())
                .and_then(|v| v.as_f64())
                .is_some_and(|n| n <= *val),
            Filter::All(filters) => filters.iter().all(|f| f.eval(props)),
            Filter::Any(filters) => filters.iter().any(|f| f.eval(props)),
            Filter::None(filters) => !filters.iter().any(|f| f.eval(props)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn props(pairs: &[(&str, PropertyValue)]) -> HashMap<Arc<str>, PropertyValue> {
        pairs
            .iter()
            .map(|(k, v)| (Arc::from(*k), v.clone()))
            .collect()
    }

    fn s(v: &str) -> PropertyValue {
        PropertyValue::String(Arc::from(v))
    }
    fn n(v: f64) -> PropertyValue {
        PropertyValue::Number(v)
    }

    #[test]
    fn test_always() {
        assert!(Filter::Always.eval(&HashMap::new()));
    }

    #[test]
    fn test_eq() {
        let p = props(&[("class", s("motorway"))]);
        assert!(Filter::Eq("class".into(), s("motorway")).eval(&p));
        assert!(!Filter::Eq("class".into(), s("street")).eval(&p));
    }

    #[test]
    fn test_in() {
        let p = props(&[("class", s("stream"))]);
        let f = Filter::In("class".into(), vec![s("stream"), s("canal")]);
        assert!(f.eval(&p));
    }

    #[test]
    fn test_gt() {
        let p = props(&[("admin_level", n(4.0))]);
        assert!(Filter::Gte("admin_level".into(), 4.0).eval(&p));
        assert!(!Filter::Gt("admin_level".into(), 4.0).eval(&p));
    }

    #[test]
    fn test_all() {
        let p = props(&[("class", s("motorway")), ("structure", s("bridge"))]);
        let f = Filter::All(vec![
            Filter::Eq("class".into(), s("motorway")),
            Filter::Eq("structure".into(), s("bridge")),
        ]);
        assert!(f.eval(&p));
    }

    #[test]
    fn test_none() {
        let p = props(&[("structure", s("none"))]);
        let f = Filter::None(vec![
            Filter::Eq("structure".into(), s("bridge")),
            Filter::Eq("structure".into(), s("tunnel")),
        ]);
        assert!(f.eval(&p));
    }
}
