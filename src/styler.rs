#![allow(clippy::type_complexity)]

use serde_json::Value;
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct FeatureStyle {
    pub id: String,
    pub style_type: String, // "line", "fill", "symbol", "background"
    pub source_layer: String,
    pub min_zoom: Option<f64>,
    pub max_zoom: Option<f64>,
    pub color: u8, // 256-color terminal code
}

pub struct Styler {
    pub name: String,
    styles_by_layer: HashMap<String, Vec<CompiledStyle>>,
    pub background_color: Option<u8>,
}

struct CompiledStyle {
    style: FeatureStyle,
    filter: Box<dyn Fn(&HashMap<String, Value>) -> bool + Send + Sync>,
}

fn replace_constants(value: &Value, constants: &HashMap<String, Value>) -> Value {
    match value {
        Value::String(s) if s.starts_with('@') => constants
            .get(s.as_str())
            .cloned()
            .unwrap_or_else(|| value.clone()),
        Value::Object(map) => {
            let new_map = map
                .iter()
                .map(|(k, v)| (k.clone(), replace_constants(v, constants)))
                .collect();
            Value::Object(new_map)
        }
        Value::Array(arr) => Value::Array(
            arr.iter()
                .map(|v| replace_constants(v, constants))
                .collect(),
        ),
        _ => value.clone(),
    }
}

fn compile_filter(filter: &Value) -> Box<dyn Fn(&HashMap<String, Value>) -> bool + Send + Sync> {
    match filter {
        Value::Null => Box::new(|_| true),
        Value::Array(arr) if arr.is_empty() => Box::new(|_| true),
        Value::Array(arr) => {
            let op = match arr.first().and_then(|v| v.as_str()) {
                Some(s) => s.to_string(),
                None => return Box::new(|_| true),
            };

            match op.as_str() {
                "==" => {
                    let key = arr
                        .get(1)
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let val = arr.get(2).cloned().unwrap_or(Value::Null);
                    Box::new(move |props| props.get(&key) == Some(&val))
                }
                "!=" => {
                    let key = arr
                        .get(1)
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let val = arr.get(2).cloned().unwrap_or(Value::Null);
                    Box::new(move |props| props.get(&key) != Some(&val))
                }
                "in" => {
                    let key = arr
                        .get(1)
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let values: Vec<Value> = arr[2..].to_vec();
                    Box::new(move |props| props.get(&key).is_some_and(|v| values.contains(v)))
                }
                "!in" => {
                    let key = arr
                        .get(1)
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let values: Vec<Value> = arr[2..].to_vec();
                    Box::new(move |props| props.get(&key).is_none_or(|v| !values.contains(v)))
                }
                "has" => {
                    let key = arr
                        .get(1)
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    Box::new(move |props| props.contains_key(&key))
                }
                "!has" => {
                    let key = arr
                        .get(1)
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    Box::new(move |props| !props.contains_key(&key))
                }
                ">" => {
                    let key = arr
                        .get(1)
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let val = arr.get(2).and_then(|v| v.as_f64()).unwrap_or(0.0);
                    Box::new(move |props| {
                        props
                            .get(&key)
                            .and_then(|v| v.as_f64())
                            .is_some_and(|n| n > val)
                    })
                }
                ">=" => {
                    let key = arr
                        .get(1)
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let val = arr.get(2).and_then(|v| v.as_f64()).unwrap_or(0.0);
                    Box::new(move |props| {
                        props
                            .get(&key)
                            .and_then(|v| v.as_f64())
                            .is_some_and(|n| n >= val)
                    })
                }
                "<" => {
                    let key = arr
                        .get(1)
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let val = arr.get(2).and_then(|v| v.as_f64()).unwrap_or(0.0);
                    Box::new(move |props| {
                        props
                            .get(&key)
                            .and_then(|v| v.as_f64())
                            .is_some_and(|n| n < val)
                    })
                }
                "<=" => {
                    let key = arr
                        .get(1)
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let val = arr.get(2).and_then(|v| v.as_f64()).unwrap_or(0.0);
                    Box::new(move |props| {
                        props
                            .get(&key)
                            .and_then(|v| v.as_f64())
                            .is_some_and(|n| n <= val)
                    })
                }
                "all" => {
                    let sub_filters: Vec<
                        Box<dyn Fn(&HashMap<String, Value>) -> bool + Send + Sync>,
                    > = arr[1..].iter().map(compile_filter).collect();
                    Box::new(move |props| sub_filters.iter().all(|f| f(props)))
                }
                "any" => {
                    let sub_filters: Vec<
                        Box<dyn Fn(&HashMap<String, Value>) -> bool + Send + Sync>,
                    > = arr[1..].iter().map(compile_filter).collect();
                    Box::new(move |props| sub_filters.iter().any(|f| f(props)))
                }
                "none" => {
                    let sub_filters: Vec<
                        Box<dyn Fn(&HashMap<String, Value>) -> bool + Send + Sync>,
                    > = arr[1..].iter().map(compile_filter).collect();
                    Box::new(move |props| !sub_filters.iter().any(|f| f(props)))
                }
                _ => Box::new(|_| true),
            }
        }
        _ => Box::new(|_| true),
    }
}

fn resolve_color(paint: &HashMap<String, Value>, keys: &[&str]) -> u8 {
    for &key in keys {
        if let Some(val) = paint.get(key) {
            match val {
                Value::String(s) => {
                    let [r, g, b] = crate::color::hex2rgb(s);
                    return crate::color::rgb_to_x256(r, g, b);
                }
                Value::Object(obj) => {
                    if let Some(Value::Array(stops)) = obj.get("stops")
                        && let Some(first_stop) = stops.first()
                        && let Some(Value::Array(stop_arr)) = Some(first_stop)
                        && let Some(Value::String(color_str)) = stop_arr.get(1)
                    {
                        let [r, g, b] = crate::color::hex2rgb(color_str);
                        return crate::color::rgb_to_x256(r, g, b);
                    }
                }
                _ => {}
            }
        }
    }
    0
}

impl Styler {
    pub fn from_json(json: &Value) -> Self {
        let name = json
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let constants: HashMap<String, Value> = json
            .get("constants")
            .and_then(|v| v.as_object())
            .map(|obj| obj.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
            .unwrap_or_default();

        let layers = json
            .get("layers")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        // Build a map of layer id -> layer for resolving `ref` fields
        let layers_by_id: HashMap<String, Value> = layers
            .iter()
            .filter_map(|l| {
                l.get("id")
                    .and_then(|v| v.as_str())
                    .map(|id| (id.to_string(), l.clone()))
            })
            .collect();

        let mut styles_by_layer: HashMap<String, Vec<CompiledStyle>> = HashMap::new();
        let mut background_color: Option<u8> = None;

        for layer in &layers {
            let layer = layer.as_object().unwrap();

            // Resolve `ref` field
            let mut layer_map = layer.clone();
            if let Some(ref_id) = layer.get("ref").and_then(|v| v.as_str())
                && let Some(ref_layer) = layers_by_id.get(ref_id).and_then(|v| v.as_object())
            {
                for field in &["type", "source-layer", "minzoom", "maxzoom", "filter"] {
                    if !layer_map.contains_key(*field)
                        && let Some(val) = ref_layer.get(*field)
                    {
                        layer_map.insert(field.to_string(), val.clone());
                    }
                }
            }

            let id = layer_map
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let style_type = layer_map
                .get("type")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let source_layer = layer_map
                .get("source-layer")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let min_zoom = layer_map.get("minzoom").and_then(|v| v.as_f64());
            let max_zoom = layer_map.get("maxzoom").and_then(|v| v.as_f64());

            // Replace constants in paint
            let raw_paint = layer_map
                .get("paint")
                .cloned()
                .unwrap_or_else(|| Value::Object(Default::default()));
            let resolved_paint = replace_constants(&raw_paint, &constants);

            let paint: HashMap<String, Value> = resolved_paint
                .as_object()
                .map(|obj| obj.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
                .unwrap_or_default();

            // Handle background type
            if style_type == "background" {
                let color = resolve_color(&paint, &["background-color"]);
                background_color = Some(color);
                continue;
            }

            let color = resolve_color(&paint, &["line-color", "fill-color", "text-color"]);

            let filter_val = layer_map.get("filter").cloned().unwrap_or(Value::Null);
            let filter = compile_filter(&filter_val);

            let feature_style = FeatureStyle {
                id,
                style_type,
                source_layer: source_layer.clone(),
                min_zoom,
                max_zoom,
                color,
            };

            styles_by_layer
                .entry(source_layer)
                .or_default()
                .push(CompiledStyle {
                    style: feature_style,
                    filter,
                });
        }

        Styler {
            name,
            styles_by_layer,
            background_color,
        }
    }

    pub fn get_style_for(
        &self,
        layer: &str,
        properties: &HashMap<String, Value>,
    ) -> Option<&FeatureStyle> {
        self.styles_by_layer
            .get(layer)?
            .iter()
            .find(|cs| (cs.filter)(properties))
            .map(|cs| &cs.style)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample_style() -> Value {
        json!({
            "name": "test",
            "constants": { "@water": "#5f87ff", "@background": "#000" },
            "layers": [
                { "type": "background", "id": "background", "paint": { "background-color": "@background" } },
                { "type": "fill", "id": "water", "paint": { "fill-color": "@water" }, "source-layer": "water" },
                { "type": "line", "id": "road_motorway", "paint": { "line-color": "#fc8" }, "source-layer": "road", "minzoom": 5, "filter": ["==", "class", "motorway"] },
                { "type": "symbol", "id": "place_city", "paint": { "text-color": "#f00" }, "source-layer": "place_label", "filter": ["==", "type", "city"] }
            ]
        })
    }

    #[test]
    fn test_parse_style() {
        let styler = Styler::from_json(&sample_style());
        assert_eq!(styler.name, "test");
        assert!(styler.background_color.is_some());
    }

    #[test]
    fn test_get_style_for_water() {
        let styler = Styler::from_json(&sample_style());
        let props = HashMap::new();
        let style = styler.get_style_for("water", &props);
        assert!(style.is_some());
        let style = style.unwrap();
        assert_eq!(style.style_type, "fill");
        assert_eq!(style.id, "water");
    }

    #[test]
    fn test_filter_match() {
        let styler = Styler::from_json(&sample_style());
        let mut props = HashMap::new();
        props.insert("class".to_string(), json!("motorway"));
        let style = styler.get_style_for("road", &props);
        assert!(style.is_some());
        assert_eq!(style.unwrap().id, "road_motorway");
    }

    #[test]
    fn test_filter_no_match() {
        let styler = Styler::from_json(&sample_style());
        let mut props = HashMap::new();
        props.insert("class".to_string(), json!("residential"));
        let style = styler.get_style_for("road", &props);
        assert!(style.is_none());
    }

    #[test]
    fn test_unknown_layer_returns_none() {
        let styler = Styler::from_json(&sample_style());
        let props = HashMap::new();
        let style = styler.get_style_for("nonexistent_layer", &props);
        assert!(style.is_none());
    }
}
