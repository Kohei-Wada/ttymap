pub mod filter;
mod preset_bright;
mod preset_dark;
#[cfg(test)]
mod preset_tests;

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
}
