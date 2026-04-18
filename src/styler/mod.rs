pub mod filter;
mod preset_bright;
mod preset_dark;

use std::collections::HashMap;

use filter::{Filter, PropertyValue};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StyleType {
    Line,
    Fill,
    Symbol,
}

#[derive(Debug, Clone)]
pub struct StyleRule {
    pub source_layer: String,
    pub style_type: StyleType,
    pub color: u8,
    pub filter: Filter,
    pub min_zoom: Option<f64>,
    pub max_zoom: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StylePreset {
    #[default]
    Dark,
    Bright,
}

pub struct Styler {
    pub name: String,
    pub(crate) rules_by_layer: HashMap<String, Vec<StyleRule>>,
    pub background_color: Option<u8>,
}

impl Styler {
    pub fn new(preset: StylePreset) -> Self {
        use crate::palette;

        let (name, p) = match preset {
            StylePreset::Dark => ("dark", &palette::DARK),
            StylePreset::Bright => ("bright", &palette::BRIGHT),
        };

        let background_color = Some(p.background);
        let rules = match preset {
            StylePreset::Dark => preset_dark::rules(p),
            StylePreset::Bright => preset_bright::rules(p),
        };

        let mut rules_by_layer: HashMap<String, Vec<StyleRule>> = HashMap::new();
        for rule in rules {
            rules_by_layer
                .entry(rule.source_layer.clone())
                .or_default()
                .push(rule);
        }

        Styler {
            name: name.to_string(),
            rules_by_layer,
            background_color,
        }
    }

    pub fn get_style_for(
        &self,
        layer: &str,
        properties: &HashMap<String, PropertyValue>,
    ) -> Option<&StyleRule> {
        self.rules_by_layer
            .get(layer)?
            .iter()
            .find(|rule| rule.filter.eval(properties))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_dark() {
        let styler = Styler::new(StylePreset::Dark);
        assert_eq!(styler.name, "dark");
        assert!(styler.background_color.is_some());
    }

    #[test]
    fn test_new_bright() {
        let styler = Styler::new(StylePreset::Bright);
        assert_eq!(styler.name, "bright");
        assert!(styler.background_color.is_some());
    }

    #[test]
    fn test_get_style_water() {
        let styler = Styler::new(StylePreset::Dark);
        let props = HashMap::new();
        let rule = styler.get_style_for("water", &props);
        assert!(rule.is_some());
        assert_eq!(rule.unwrap().style_type, StyleType::Fill);
    }

    #[test]
    fn test_filter_match_road() {
        let styler = Styler::new(StylePreset::Dark);
        let mut props = HashMap::new();
        props.insert("class".into(), PropertyValue::String("motorway".into()));
        props.insert("$type".into(), PropertyValue::String("LineString".into()));
        let rule = styler.get_style_for("road", &props);
        assert!(rule.is_some());
    }

    #[test]
    fn test_unknown_layer() {
        let styler = Styler::new(StylePreset::Dark);
        let props = HashMap::new();
        assert!(styler.get_style_for("nonexistent", &props).is_none());
    }

    // --- Dark preset tests ---

    #[test]
    fn dark_has_water_fill() {
        let styler = Styler::new(StylePreset::Dark);
        let props = HashMap::new();
        let rule = styler.get_style_for("water", &props).unwrap();
        assert_eq!(rule.style_type, StyleType::Fill);
    }

    #[test]
    fn dark_road_motorway_matches() {
        let styler = Styler::new(StylePreset::Dark);
        let mut props = HashMap::new();
        props.insert("class".into(), PropertyValue::String("motorway".into()));
        props.insert("$type".into(), PropertyValue::String("LineString".into()));
        let rule = styler.get_style_for("road", &props).unwrap();
        assert_eq!(rule.style_type, StyleType::Line);
    }

    #[test]
    fn dark_road_tunnel_motorway_matches() {
        let styler = Styler::new(StylePreset::Dark);
        let mut props = HashMap::new();
        props.insert("class".into(), PropertyValue::String("motorway".into()));
        props.insert("structure".into(), PropertyValue::String("tunnel".into()));
        let rule = styler.get_style_for("road", &props).unwrap();
        assert_eq!(rule.style_type, StyleType::Line);
    }

    #[test]
    fn dark_admin_level_2() {
        let styler = Styler::new(StylePreset::Dark);
        let mut props = HashMap::new();
        props.insert("admin_level".into(), PropertyValue::Number(2.0));
        props.insert("disputed".into(), PropertyValue::Number(0.0));
        props.insert("maritime".into(), PropertyValue::Number(0.0));
        let rule = styler.get_style_for("admin", &props).unwrap();
        assert_eq!(rule.style_type, StyleType::Line);
    }

    #[test]
    fn dark_place_label_city() {
        let styler = Styler::new(StylePreset::Dark);
        let mut props = HashMap::new();
        props.insert("type".into(), PropertyValue::String("city".into()));
        let rule = styler.get_style_for("place_label", &props).unwrap();
        assert_eq!(rule.style_type, StyleType::Symbol);
    }

    #[test]
    fn dark_aeroway_runway() {
        let styler = Styler::new(StylePreset::Dark);
        let mut props = HashMap::new();
        props.insert("$type".into(), PropertyValue::String("LineString".into()));
        props.insert("type".into(), PropertyValue::String("runway".into()));
        let rule = styler.get_style_for("aeroway", &props).unwrap();
        assert_eq!(rule.style_type, StyleType::Line);
        assert_eq!(rule.min_zoom, Some(11.0));
    }

    #[test]
    fn dark_building() {
        let styler = Styler::new(StylePreset::Dark);
        let props = HashMap::new();
        let rule = styler.get_style_for("building", &props).unwrap();
        assert_eq!(rule.style_type, StyleType::Line);
    }

    #[test]
    fn dark_poi_label_scalerank_filter() {
        let styler = Styler::new(StylePreset::Dark);
        let mut props = HashMap::new();
        props.insert("$type".into(), PropertyValue::String("Point".into()));
        props.insert("scalerank".into(), PropertyValue::Number(1.0));
        let rule = styler.get_style_for("poi_label", &props).unwrap();
        assert_eq!(rule.style_type, StyleType::Symbol);
    }

    // --- Bright preset tests ---

    #[test]
    fn bright_has_water_fill() {
        let styler = Styler::new(StylePreset::Bright);
        let props = HashMap::new();
        let rule = styler.get_style_for("water", &props).unwrap();
        assert_eq!(rule.style_type, StyleType::Fill);
    }

    #[test]
    fn bright_road_motorway_matches() {
        let styler = Styler::new(StylePreset::Bright);
        let mut props = HashMap::new();
        props.insert("class".into(), PropertyValue::String("motorway".into()));
        props.insert("$type".into(), PropertyValue::String("LineString".into()));
        let rule = styler.get_style_for("road", &props).unwrap();
        assert_eq!(rule.style_type, StyleType::Line);
    }

    #[test]
    fn bright_building_has_minzoom() {
        let styler = Styler::new(StylePreset::Bright);
        let props = HashMap::new();
        let rule = styler.get_style_for("building", &props).unwrap();
        assert_eq!(rule.min_zoom, Some(14.5));
    }

    #[test]
    fn bright_housenum_label() {
        let styler = Styler::new(StylePreset::Bright);
        let props = HashMap::new();
        let rule = styler.get_style_for("housenum_label", &props).unwrap();
        assert_eq!(rule.style_type, StyleType::Symbol);
    }

    // --- Cross-preset tests ---

    #[test]
    fn both_presets_have_background_color() {
        let dark = Styler::new(StylePreset::Dark);
        let bright = Styler::new(StylePreset::Bright);
        assert!(dark.background_color.is_some());
        assert!(bright.background_color.is_some());
    }

    #[test]
    fn unknown_layer_returns_none_both_presets() {
        let dark = Styler::new(StylePreset::Dark);
        let bright = Styler::new(StylePreset::Bright);
        let props = HashMap::new();
        assert!(dark.get_style_for("nonexistent", &props).is_none());
        assert!(bright.get_style_for("nonexistent", &props).is_none());
    }

    #[test]
    fn all_draw_order_layers_have_rules() {
        // Layers common to both presets
        let common_layers = vec![
            "landuse",
            "water",
            "waterway",
            "marine_label",
            "aeroway",
            "building",
            "road",
            "admin",
            "country_label",
            "water_label",
            "place_label",
            "rail_station_label",
            "airport_label",
            "poi_label",
            "road_label",
        ];

        for preset in [StylePreset::Dark, StylePreset::Bright] {
            let styler = Styler::new(preset);
            for layer in &common_layers {
                assert!(
                    styler.rules_by_layer.contains_key(*layer),
                    "Preset {:?} missing rules for layer '{}'",
                    preset,
                    layer
                );
            }
        }

        // Dark-only layers
        let dark_styler = Styler::new(StylePreset::Dark);
        assert!(dark_styler.rules_by_layer.contains_key("landuse_overlay"));

        // Bright-only layers
        let bright_styler = Styler::new(StylePreset::Bright);
        assert!(bright_styler.rules_by_layer.contains_key("housenum_label"));
    }
}
