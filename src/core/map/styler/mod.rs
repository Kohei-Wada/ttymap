pub mod filter;
mod schema;

use std::collections::HashMap;
use std::sync::Arc;

use filter::Filter;

use crate::core::map::tile::PropertyValue;

use crate::theme::{ColorPalette, ThemeId};

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

pub struct Styler {
    theme: ThemeId,
    pub(crate) rules_by_layer: HashMap<String, Vec<StyleRule>>,
    pub background_color: Option<u8>,
}

impl Styler {
    /// Build a styler for the given theme. Pair with
    /// [`ThemeId::palette`] to feed the same color set into the UI
    /// layer.
    pub fn new(theme: ThemeId) -> Self {
        let p = theme.palette();
        let rules = schema::mapscii::rules(p);

        let mut rules_by_layer: HashMap<String, Vec<StyleRule>> = HashMap::new();
        for rule in rules {
            rules_by_layer
                .entry(rule.source_layer.clone())
                .or_default()
                .push(rule);
        }

        Styler {
            theme,
            rules_by_layer,
            background_color: Some(p.background),
        }
    }

    pub fn theme(&self) -> ThemeId {
        self.theme
    }

    pub fn name(&self) -> &'static str {
        self.theme.name()
    }

    /// Convenience accessor — equivalent to `self.theme().palette()`.
    pub fn palette(&self) -> &'static ColorPalette {
        self.theme.palette()
    }

    pub fn get_style_for(
        &self,
        layer: &str,
        properties: &HashMap<Arc<str>, PropertyValue>,
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
        let styler = Styler::new(ThemeId::Dark);
        assert_eq!(styler.name(), "dark");
        assert!(styler.background_color.is_some());
    }

    #[test]
    fn test_new_bright() {
        let styler = Styler::new(ThemeId::Bright);
        assert_eq!(styler.name(), "bright");
        assert!(styler.background_color.is_some());
    }

    #[test]
    fn test_get_style_water() {
        let styler = Styler::new(ThemeId::Dark);
        let props = HashMap::new();
        let rule = styler.get_style_for("water", &props);
        assert!(rule.is_some());
        assert_eq!(rule.unwrap().style_type, StyleType::Fill);
    }

    #[test]
    fn test_filter_match_road() {
        let styler = Styler::new(ThemeId::Dark);
        let mut props = HashMap::new();
        props.insert(
            Arc::from("class"),
            PropertyValue::String(Arc::from("motorway")),
        );
        props.insert(
            Arc::from("$type"),
            PropertyValue::String(Arc::from("LineString")),
        );
        let rule = styler.get_style_for("road", &props);
        assert!(rule.is_some());
    }

    #[test]
    fn test_unknown_layer() {
        let styler = Styler::new(ThemeId::Dark);
        let props = HashMap::new();
        assert!(styler.get_style_for("nonexistent", &props).is_none());
    }

    // --- Dark preset tests ---

    #[test]
    fn dark_has_water_fill() {
        let styler = Styler::new(ThemeId::Dark);
        let props = HashMap::new();
        let rule = styler.get_style_for("water", &props).unwrap();
        assert_eq!(rule.style_type, StyleType::Fill);
    }

    #[test]
    fn dark_road_motorway_matches() {
        let styler = Styler::new(ThemeId::Dark);
        let mut props = HashMap::new();
        props.insert(
            Arc::from("class"),
            PropertyValue::String(Arc::from("motorway")),
        );
        props.insert(
            Arc::from("$type"),
            PropertyValue::String(Arc::from("LineString")),
        );
        let rule = styler.get_style_for("road", &props).unwrap();
        assert_eq!(rule.style_type, StyleType::Line);
    }

    #[test]
    fn dark_road_tunnel_motorway_matches() {
        let styler = Styler::new(ThemeId::Dark);
        let mut props = HashMap::new();
        props.insert("class".into(), PropertyValue::String("motorway".into()));
        props.insert("structure".into(), PropertyValue::String("tunnel".into()));
        let rule = styler.get_style_for("road", &props).unwrap();
        assert_eq!(rule.style_type, StyleType::Line);
    }

    #[test]
    fn dark_admin_level_2() {
        let styler = Styler::new(ThemeId::Dark);
        let mut props = HashMap::new();
        props.insert("admin_level".into(), PropertyValue::Number(2.0));
        props.insert("disputed".into(), PropertyValue::Number(0.0));
        props.insert("maritime".into(), PropertyValue::Number(0.0));
        let rule = styler.get_style_for("admin", &props).unwrap();
        assert_eq!(rule.style_type, StyleType::Line);
    }

    #[test]
    fn dark_place_label_city() {
        let styler = Styler::new(ThemeId::Dark);
        let mut props = HashMap::new();
        props.insert("type".into(), PropertyValue::String("city".into()));
        let rule = styler.get_style_for("place_label", &props).unwrap();
        assert_eq!(rule.style_type, StyleType::Symbol);
    }

    #[test]
    fn dark_aeroway_runway() {
        let styler = Styler::new(ThemeId::Dark);
        let mut props = HashMap::new();
        props.insert("$type".into(), PropertyValue::String("LineString".into()));
        props.insert("type".into(), PropertyValue::String("runway".into()));
        let rule = styler.get_style_for("aeroway", &props).unwrap();
        assert_eq!(rule.style_type, StyleType::Line);
        assert_eq!(rule.min_zoom, Some(11.0));
    }

    #[test]
    fn dark_building() {
        let styler = Styler::new(ThemeId::Dark);
        let props = HashMap::new();
        let rule = styler.get_style_for("building", &props).unwrap();
        assert_eq!(rule.style_type, StyleType::Line);
    }

    #[test]
    fn dark_poi_label_scalerank_filter() {
        let styler = Styler::new(ThemeId::Dark);
        let mut props = HashMap::new();
        props.insert("$type".into(), PropertyValue::String("Point".into()));
        props.insert("scalerank".into(), PropertyValue::Number(1.0));
        let rule = styler.get_style_for("poi_label", &props).unwrap();
        assert_eq!(rule.style_type, StyleType::Symbol);
    }

    // --- Bright preset tests ---

    #[test]
    fn bright_has_water_fill() {
        let styler = Styler::new(ThemeId::Bright);
        let props = HashMap::new();
        let rule = styler.get_style_for("water", &props).unwrap();
        assert_eq!(rule.style_type, StyleType::Fill);
    }

    #[test]
    fn bright_road_motorway_matches() {
        let styler = Styler::new(ThemeId::Bright);
        let mut props = HashMap::new();
        props.insert(
            Arc::from("class"),
            PropertyValue::String(Arc::from("motorway")),
        );
        props.insert(
            Arc::from("$type"),
            PropertyValue::String(Arc::from("LineString")),
        );
        let rule = styler.get_style_for("road", &props).unwrap();
        assert_eq!(rule.style_type, StyleType::Line);
    }

    #[test]
    fn bright_building_has_minzoom() {
        let styler = Styler::new(ThemeId::Bright);
        let props = HashMap::new();
        let rule = styler.get_style_for("building", &props).unwrap();
        assert_eq!(rule.min_zoom, Some(14.5));
    }

    #[test]
    fn bright_housenum_label() {
        let styler = Styler::new(ThemeId::Bright);
        let props = HashMap::new();
        let rule = styler.get_style_for("housenum_label", &props).unwrap();
        assert_eq!(rule.style_type, StyleType::Symbol);
    }

    // --- Cross-preset tests ---

    #[test]
    fn both_presets_have_background_color() {
        let dark = Styler::new(ThemeId::Dark);
        let bright = Styler::new(ThemeId::Bright);
        assert!(dark.background_color.is_some());
        assert!(bright.background_color.is_some());
    }

    #[test]
    fn unknown_layer_returns_none_both_presets() {
        let dark = Styler::new(ThemeId::Dark);
        let bright = Styler::new(ThemeId::Bright);
        let props = HashMap::new();
        assert!(dark.get_style_for("nonexistent", &props).is_none());
        assert!(bright.get_style_for("nonexistent", &props).is_none());
    }

    #[test]
    fn all_draw_order_layers_have_rules() {
        // Both themes share the same rule shape (single source of truth
        // in `schema/mapscii.rs`); the only difference is `ColorPalette`.
        let layers = vec![
            "landuse",
            "landuse_overlay",
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
            "housenum_label",
        ];

        for theme in [ThemeId::Dark, ThemeId::Bright] {
            let styler = Styler::new(theme);
            let name = theme.name();
            for layer in &layers {
                assert!(
                    styler.rules_by_layer.contains_key(*layer),
                    "Preset {:?} missing rules for layer '{}'",
                    name,
                    layer
                );
            }
        }
    }

    #[test]
    fn both_themes_have_identical_layer_set_and_rule_count() {
        // Structural guard: schema/theme orthogonality means every rule
        // exists in both themes. If this assertion fires, someone added a
        // theme-conditional rule path — see `docs/design.md`.
        let dark = Styler::new(ThemeId::Dark);
        let bright = Styler::new(ThemeId::Bright);

        let dark_layers: std::collections::BTreeSet<_> = dark.rules_by_layer.keys().collect();
        let bright_layers: std::collections::BTreeSet<_> = bright.rules_by_layer.keys().collect();
        assert_eq!(dark_layers, bright_layers, "layer sets diverged");

        for layer in dark_layers {
            assert_eq!(
                dark.rules_by_layer[layer.as_str()].len(),
                bright.rules_by_layer[layer.as_str()].len(),
                "rule count differs for layer '{}'",
                layer
            );
        }
    }
}
