use crate::color_palette::ColorPalette;

use super::filter::Filter;
use super::{StyleRule, StyleType};
use crate::map::tile::PropertyValue as PV;

fn s(v: &str) -> PV {
    PV::String(std::sync::Arc::from(v))
}

fn n(v: f64) -> PV {
    PV::Number(v)
}

pub fn rules(p: &ColorPalette) -> Vec<StyleRule> {
    vec![
        // 1. landuse_overlay_national_park
        StyleRule {
            source_layer: "landuse_overlay".into(),
            style_type: StyleType::Fill,
            color: p.landuse_overlay,
            filter: Filter::Eq("class".into(), s("national_park")),
            min_zoom: None,
            max_zoom: None,
        },
        // 2. landuse_park
        StyleRule {
            source_layer: "landuse".into(),
            style_type: StyleType::Fill,
            color: p.landuse_park,
            filter: Filter::Eq("class".into(), s("park")),
            min_zoom: None,
            max_zoom: None,
        },
        // 3. landuse_cemetery
        StyleRule {
            source_layer: "landuse".into(),
            style_type: StyleType::Line,
            color: p.landuse_cemetery,
            filter: Filter::Eq("class".into(), s("cemetery")),
            min_zoom: None,
            max_zoom: None,
        },
        // 4. landuse_hospital
        StyleRule {
            source_layer: "landuse".into(),
            style_type: StyleType::Fill,
            color: p.landuse_hospital,
            filter: Filter::Eq("class".into(), s("hospital")),
            min_zoom: None,
            max_zoom: None,
        },
        // 5. landuse_school
        StyleRule {
            source_layer: "landuse".into(),
            style_type: StyleType::Line,
            color: p.landuse_school,
            filter: Filter::Eq("class".into(), s("school")),
            min_zoom: None,
            max_zoom: None,
        },
        // 6. landuse_wood
        StyleRule {
            source_layer: "landuse".into(),
            style_type: StyleType::Line,
            color: p.landuse_wood,
            filter: Filter::Eq("class".into(), s("wood")),
            min_zoom: None,
            max_zoom: None,
        },
        // 2. waterway
        StyleRule {
            source_layer: "waterway".into(),
            style_type: StyleType::Line,
            color: p.waterway_deep,
            filter: Filter::All(vec![
                Filter::NotEq("class".into(), s("river")),
                Filter::NotEq("class".into(), s("stream")),
                Filter::NotEq("class".into(), s("canal")),
            ]),
            min_zoom: None,
            max_zoom: None,
        },
        // 3. waterway_river
        StyleRule {
            source_layer: "waterway".into(),
            style_type: StyleType::Line,
            color: p.waterway_deep,
            filter: Filter::Eq("class".into(), s("river")),
            min_zoom: None,
            max_zoom: None,
        },
        // 4. waterway_stream_canal
        StyleRule {
            source_layer: "waterway".into(),
            style_type: StyleType::Line,
            color: p.waterway,
            filter: Filter::In("class".into(), vec![s("stream"), s("canal")]),
            min_zoom: None,
            max_zoom: None,
        },
        // 5. water
        StyleRule {
            source_layer: "water".into(),
            style_type: StyleType::Fill,
            color: p.water,
            filter: Filter::Always,
            min_zoom: None,
            max_zoom: None,
        },
        // 6. aeroway_fill
        StyleRule {
            source_layer: "aeroway".into(),
            style_type: StyleType::Fill,
            color: p.aeroway,
            filter: Filter::Eq("$type".into(), s("Polygon")),
            min_zoom: Some(11.0),
            max_zoom: None,
        },
        // 7. aeroway_runway
        StyleRule {
            source_layer: "aeroway".into(),
            style_type: StyleType::Line,
            color: p.aeroway,
            filter: Filter::All(vec![
                Filter::Eq("$type".into(), s("LineString")),
                Filter::Eq("type".into(), s("runway")),
            ]),
            min_zoom: Some(11.0),
            max_zoom: None,
        },
        // 8. aeroway_taxiway
        StyleRule {
            source_layer: "aeroway".into(),
            style_type: StyleType::Line,
            color: p.aeroway,
            filter: Filter::All(vec![
                Filter::Eq("$type".into(), s("LineString")),
                Filter::Eq("type".into(), s("taxiway")),
            ]),
            min_zoom: Some(11.0),
            max_zoom: None,
        },
        // 9. building
        StyleRule {
            source_layer: "building".into(),
            style_type: StyleType::Line,
            color: p.building,
            filter: Filter::Always,
            min_zoom: Some(14.5),
            max_zoom: None,
        },
        // 10. tunnel_motorway_link_casing
        StyleRule {
            source_layer: "road".into(),
            style_type: StyleType::Line,
            color: p.road_casing_major,
            filter: Filter::All(vec![
                Filter::Eq("structure".into(), s("tunnel")),
                Filter::Eq("class".into(), s("motorway_link")),
            ]),
            min_zoom: None,
            max_zoom: None,
        },
        // 11. tunnel_service_track_casing
        StyleRule {
            source_layer: "road".into(),
            style_type: StyleType::Line,
            color: p.road_casing_minor,
            filter: Filter::All(vec![
                Filter::Eq("structure".into(), s("tunnel")),
                Filter::In("class".into(), vec![s("service"), s("track")]),
            ]),
            min_zoom: None,
            max_zoom: None,
        },
        // 12. tunnel_link_casing
        StyleRule {
            source_layer: "road".into(),
            style_type: StyleType::Line,
            color: p.road_casing_major,
            filter: Filter::All(vec![
                Filter::Eq("structure".into(), s("tunnel")),
                Filter::Eq("class".into(), s("link")),
            ]),
            min_zoom: None,
            max_zoom: None,
        },
        // 13. tunnel_street_casing
        StyleRule {
            source_layer: "road".into(),
            style_type: StyleType::Line,
            color: p.road_casing_minor,
            filter: Filter::All(vec![
                Filter::Eq("structure".into(), s("tunnel")),
                Filter::In("class".into(), vec![s("street"), s("street_limited")]),
            ]),
            min_zoom: None,
            max_zoom: None,
        },
        // 14. tunnel_secondary_tertiary_casing
        StyleRule {
            source_layer: "road".into(),
            style_type: StyleType::Line,
            color: p.road_casing_major,
            filter: Filter::All(vec![
                Filter::Eq("structure".into(), s("tunnel")),
                Filter::In("class".into(), vec![s("secondary"), s("tertiary")]),
            ]),
            min_zoom: None,
            max_zoom: None,
        },
        // 15. tunnel_trunk_primary_casing
        StyleRule {
            source_layer: "road".into(),
            style_type: StyleType::Line,
            color: p.road_casing_major,
            filter: Filter::All(vec![
                Filter::Eq("structure".into(), s("tunnel")),
                Filter::In("class".into(), vec![s("trunk"), s("primary")]),
            ]),
            min_zoom: None,
            max_zoom: None,
        },
        // 16. tunnel_motorway_casing
        StyleRule {
            source_layer: "road".into(),
            style_type: StyleType::Line,
            color: p.road_casing_major,
            filter: Filter::All(vec![
                Filter::Eq("structure".into(), s("tunnel")),
                Filter::Eq("class".into(), s("motorway")),
            ]),
            min_zoom: None,
            max_zoom: None,
        },
        // 17. tunnel_path_pedestrian
        StyleRule {
            source_layer: "road".into(),
            style_type: StyleType::Line,
            color: p.road_path_pedestrian,
            filter: Filter::All(vec![
                Filter::Eq("$type".into(), s("LineString")),
                Filter::All(vec![
                    Filter::Eq("structure".into(), s("tunnel")),
                    Filter::In("class".into(), vec![s("path"), s("pedestrian")]),
                ]),
            ]),
            min_zoom: None,
            max_zoom: None,
        },
        // 18. tunnel_major_rail
        StyleRule {
            source_layer: "road".into(),
            style_type: StyleType::Line,
            color: p.road_rail,
            filter: Filter::All(vec![
                Filter::Eq("structure".into(), s("tunnel")),
                Filter::In("class".into(), vec![s("major_rail"), s("minor_rail")]),
            ]),
            min_zoom: None,
            max_zoom: None,
        },
        // 19. road_motorway_link_casing
        StyleRule {
            source_layer: "road".into(),
            style_type: StyleType::Line,
            color: p.road_casing_major,
            filter: Filter::All(vec![
                Filter::Eq("class".into(), s("motorway_link")),
                Filter::NotIn("structure".into(), vec![s("bridge"), s("tunnel")]),
            ]),
            min_zoom: Some(12.0),
            max_zoom: None,
        },
        // 20. road_service_track_casing
        StyleRule {
            source_layer: "road".into(),
            style_type: StyleType::Line,
            color: p.road_casing_minor,
            filter: Filter::All(vec![
                Filter::In("class".into(), vec![s("service"), s("track")]),
                Filter::NotIn("structure".into(), vec![s("bridge"), s("tunnel")]),
            ]),
            min_zoom: None,
            max_zoom: None,
        },
        // 21. road_link_casing
        StyleRule {
            source_layer: "road".into(),
            style_type: StyleType::Line,
            color: p.road_casing_major,
            filter: Filter::All(vec![
                Filter::Eq("class".into(), s("link")),
                Filter::NotIn("structure".into(), vec![s("bridge"), s("tunnel")]),
            ]),
            min_zoom: Some(13.0),
            max_zoom: None,
        },
        // 22. road_street_casing
        StyleRule {
            source_layer: "road".into(),
            style_type: StyleType::Line,
            color: p.road_casing_minor,
            filter: Filter::All(vec![
                Filter::Eq("$type".into(), s("LineString")),
                Filter::All(vec![
                    Filter::In("class".into(), vec![s("street"), s("street_limited")]),
                    Filter::NotIn("structure".into(), vec![s("bridge"), s("tunnel")]),
                ]),
            ]),
            min_zoom: None,
            max_zoom: None,
        },
        // 23. road_secondary_tertiary_casing
        StyleRule {
            source_layer: "road".into(),
            style_type: StyleType::Line,
            color: p.road_casing_major,
            filter: Filter::All(vec![
                Filter::In("class".into(), vec![s("secondary"), s("tertiary")]),
                Filter::NotIn("structure".into(), vec![s("bridge"), s("tunnel")]),
            ]),
            min_zoom: None,
            max_zoom: None,
        },
        // 24. road_trunk_primary_casing
        StyleRule {
            source_layer: "road".into(),
            style_type: StyleType::Line,
            color: p.road_casing_major,
            filter: Filter::All(vec![
                Filter::In("class".into(), vec![s("trunk"), s("primary")]),
                Filter::NotIn("structure".into(), vec![s("bridge"), s("tunnel")]),
            ]),
            min_zoom: None,
            max_zoom: None,
        },
        // 25. road_motorway_casing
        StyleRule {
            source_layer: "road".into(),
            style_type: StyleType::Line,
            color: p.road_casing_major,
            filter: Filter::All(vec![
                Filter::Eq("class".into(), s("motorway")),
                Filter::NotIn("structure".into(), vec![s("bridge"), s("tunnel")]),
            ]),
            min_zoom: Some(5.0),
            max_zoom: None,
        },
        // 26. road_path_pedestrian
        StyleRule {
            source_layer: "road".into(),
            style_type: StyleType::Line,
            color: p.road_path_pedestrian,
            filter: Filter::All(vec![
                Filter::Eq("$type".into(), s("LineString")),
                Filter::All(vec![
                    Filter::In("class".into(), vec![s("path"), s("pedestrian")]),
                    Filter::NotIn("structure".into(), vec![s("bridge"), s("tunnel")]),
                ]),
            ]),
            min_zoom: None,
            max_zoom: None,
        },
        // 27. road_major_rail
        StyleRule {
            source_layer: "road".into(),
            style_type: StyleType::Line,
            color: p.road_rail,
            filter: Filter::All(vec![
                Filter::Eq("class".into(), s("major_rail")),
                Filter::NotIn("structure".into(), vec![s("bridge"), s("tunnel")]),
            ]),
            min_zoom: None,
            max_zoom: None,
        },
        // 28. bridge_motorway_link_casing
        StyleRule {
            source_layer: "road".into(),
            style_type: StyleType::Line,
            color: p.road_casing_major,
            filter: Filter::All(vec![
                Filter::Eq("structure".into(), s("bridge")),
                Filter::Eq("class".into(), s("motorway_link")),
            ]),
            min_zoom: None,
            max_zoom: None,
        },
        // 29. bridge_service_track_casing
        StyleRule {
            source_layer: "road".into(),
            style_type: StyleType::Line,
            color: p.road_casing_minor,
            filter: Filter::All(vec![
                Filter::Eq("structure".into(), s("bridge")),
                Filter::In("class".into(), vec![s("service"), s("track")]),
            ]),
            min_zoom: None,
            max_zoom: None,
        },
        // 30. bridge_link_casing
        StyleRule {
            source_layer: "road".into(),
            style_type: StyleType::Line,
            color: p.road_casing_major,
            filter: Filter::All(vec![
                Filter::Eq("structure".into(), s("bridge")),
                Filter::Eq("class".into(), s("link")),
            ]),
            min_zoom: None,
            max_zoom: None,
        },
        // 31. bridge_street_casing
        StyleRule {
            source_layer: "road".into(),
            style_type: StyleType::Line,
            color: p.road_casing_minor,
            filter: Filter::All(vec![
                Filter::Eq("structure".into(), s("bridge")),
                Filter::In("class".into(), vec![s("street"), s("street_limited")]),
            ]),
            min_zoom: None,
            max_zoom: None,
        },
        // 32. bridge_secondary_tertiary_casing
        StyleRule {
            source_layer: "road".into(),
            style_type: StyleType::Line,
            color: p.road_casing_major,
            filter: Filter::All(vec![
                Filter::Eq("structure".into(), s("bridge")),
                Filter::In("class".into(), vec![s("secondary"), s("tertiary")]),
            ]),
            min_zoom: None,
            max_zoom: None,
        },
        // 33. bridge_trunk_primary_casing
        StyleRule {
            source_layer: "road".into(),
            style_type: StyleType::Line,
            color: p.road_casing_major,
            filter: Filter::All(vec![
                Filter::Eq("structure".into(), s("bridge")),
                Filter::In("class".into(), vec![s("trunk"), s("primary")]),
            ]),
            min_zoom: None,
            max_zoom: None,
        },
        // 34. bridge_motorway_casing
        StyleRule {
            source_layer: "road".into(),
            style_type: StyleType::Line,
            color: p.road_casing_major,
            filter: Filter::All(vec![
                Filter::Eq("structure".into(), s("bridge")),
                Filter::Eq("class".into(), s("motorway")),
            ]),
            min_zoom: None,
            max_zoom: None,
        },
        // 35. bridge_path_pedestrian
        StyleRule {
            source_layer: "road".into(),
            style_type: StyleType::Line,
            color: p.road_path_pedestrian,
            filter: Filter::All(vec![
                Filter::Eq("$type".into(), s("LineString")),
                Filter::All(vec![
                    Filter::Eq("structure".into(), s("bridge")),
                    Filter::In("class".into(), vec![s("path"), s("pedestrian")]),
                ]),
            ]),
            min_zoom: None,
            max_zoom: None,
        },
        // 36. bridge_major_rail
        StyleRule {
            source_layer: "road".into(),
            style_type: StyleType::Line,
            color: p.road_rail,
            filter: Filter::All(vec![
                Filter::Eq("structure".into(), s("bridge")),
                Filter::Eq("class".into(), s("major_rail")),
            ]),
            min_zoom: None,
            max_zoom: None,
        },
        // 37. admin_level_4
        StyleRule {
            source_layer: "admin".into(),
            style_type: StyleType::Line,
            color: p.admin_level_4,
            filter: Filter::All(vec![
                Filter::Gte("admin_level".into(), 4.0),
                Filter::Eq("maritime".into(), n(0.0)),
            ]),
            min_zoom: None,
            max_zoom: None,
        },
        // 38. admin_level_3
        StyleRule {
            source_layer: "admin".into(),
            style_type: StyleType::Line,
            color: p.admin_level_3,
            filter: Filter::All(vec![
                Filter::Eq("admin_level".into(), n(3.0)),
                Filter::Eq("maritime".into(), n(0.0)),
            ]),
            min_zoom: None,
            max_zoom: None,
        },
        // 39. admin_level_2
        StyleRule {
            source_layer: "admin".into(),
            style_type: StyleType::Line,
            color: p.admin_level_2,
            filter: Filter::All(vec![
                Filter::Eq("admin_level".into(), n(2.0)),
                Filter::Eq("disputed".into(), n(0.0)),
                Filter::Eq("maritime".into(), n(0.0)),
            ]),
            min_zoom: None,
            max_zoom: None,
        },
        // 40. admin_level_2_disputed
        StyleRule {
            source_layer: "admin".into(),
            style_type: StyleType::Line,
            color: p.admin_disputed,
            filter: Filter::All(vec![
                Filter::Eq("admin_level".into(), n(2.0)),
                Filter::Eq("disputed".into(), n(1.0)),
                Filter::Eq("maritime".into(), n(0.0)),
            ]),
            min_zoom: None,
            max_zoom: None,
        },
        // 41. water_label
        StyleRule {
            source_layer: "water_label".into(),
            style_type: StyleType::Symbol,
            color: p.water_label,
            filter: Filter::Eq("$type".into(), s("Point")),
            min_zoom: None,
            max_zoom: None,
        },
        // 42. poi_label_4
        StyleRule {
            source_layer: "poi_label".into(),
            style_type: StyleType::Symbol,
            color: p.poi_label_4,
            filter: Filter::All(vec![
                Filter::Eq("$type".into(), s("Point")),
                Filter::Eq("scalerank".into(), n(4.0)),
            ]),
            min_zoom: Some(16.0),
            max_zoom: None,
        },
        // 43. poi_label_3
        StyleRule {
            source_layer: "poi_label".into(),
            style_type: StyleType::Symbol,
            color: p.poi_label_3,
            filter: Filter::All(vec![
                Filter::Eq("$type".into(), s("Point")),
                Filter::Eq("scalerank".into(), n(3.0)),
            ]),
            min_zoom: Some(15.0),
            max_zoom: None,
        },
        // 44. poi_label_2
        StyleRule {
            source_layer: "poi_label".into(),
            style_type: StyleType::Symbol,
            color: p.poi_label_2,
            filter: Filter::All(vec![
                Filter::Eq("$type".into(), s("Point")),
                Filter::Eq("scalerank".into(), n(2.0)),
            ]),
            min_zoom: Some(14.0),
            max_zoom: None,
        },
        // 45. rail_station_label
        StyleRule {
            source_layer: "rail_station_label".into(),
            style_type: StyleType::Symbol,
            color: p.rail_station_label,
            filter: Filter::Always,
            min_zoom: None,
            max_zoom: None,
        },
        // 46. poi_label_1
        StyleRule {
            source_layer: "poi_label".into(),
            style_type: StyleType::Symbol,
            color: p.poi_label_1,
            filter: Filter::All(vec![
                Filter::Eq("$type".into(), s("Point")),
                Filter::Eq("scalerank".into(), n(1.0)),
            ]),
            min_zoom: Some(13.0),
            max_zoom: None,
        },
        // 47. airport_label
        StyleRule {
            source_layer: "airport_label".into(),
            style_type: StyleType::Symbol,
            color: p.airport_label,
            filter: Filter::All(vec![
                Filter::Eq("$type".into(), s("Point")),
                Filter::In("scalerank".into(), vec![n(1.0), n(2.0), n(3.0)]),
            ]),
            min_zoom: Some(11.0),
            max_zoom: None,
        },
        // 48. road_label
        StyleRule {
            source_layer: "road_label".into(),
            style_type: StyleType::Symbol,
            color: p.road_label,
            filter: Filter::NotEq("class".into(), s("ferry")),
            min_zoom: None,
            max_zoom: None,
        },
        // 49. housenum_label
        StyleRule {
            source_layer: "housenum_label".into(),
            style_type: StyleType::Symbol,
            color: p.housenum_label,
            filter: Filter::Always,
            min_zoom: Some(16.5),
            max_zoom: None,
        },
        // 50. place_label_other
        StyleRule {
            source_layer: "place_label".into(),
            style_type: StyleType::Symbol,
            color: p.place_other,
            filter: Filter::In(
                "type".into(),
                vec![
                    s("hamlet"),
                    s("suburb"),
                    s("neighbourhood"),
                    s("island"),
                    s("islet"),
                ],
            ),
            min_zoom: None,
            max_zoom: None,
        },
        // 51. place_label_village
        StyleRule {
            source_layer: "place_label".into(),
            style_type: StyleType::Symbol,
            color: p.place_village,
            filter: Filter::Eq("type".into(), s("village")),
            min_zoom: None,
            max_zoom: None,
        },
        // 52. place_label_town
        StyleRule {
            source_layer: "place_label".into(),
            style_type: StyleType::Symbol,
            color: p.place_town,
            filter: Filter::Eq("type".into(), s("town")),
            min_zoom: None,
            max_zoom: None,
        },
        // 53. place_label_city
        StyleRule {
            source_layer: "place_label".into(),
            style_type: StyleType::Symbol,
            color: p.place_city,
            filter: Filter::Eq("type".into(), s("city")),
            min_zoom: None,
            max_zoom: None,
        },
        // 54. marine_label_line_4
        StyleRule {
            source_layer: "marine_label".into(),
            style_type: StyleType::Symbol,
            color: p.marine_label,
            filter: Filter::All(vec![
                Filter::Eq("$type".into(), s("LineString")),
                Filter::Gte("labelrank".into(), 4.0),
            ]),
            min_zoom: None,
            max_zoom: None,
        },
        // 55. marine_label_4
        StyleRule {
            source_layer: "marine_label".into(),
            style_type: StyleType::Symbol,
            color: p.marine_label,
            filter: Filter::All(vec![
                Filter::Eq("$type".into(), s("Point")),
                Filter::Gte("labelrank".into(), 4.0),
            ]),
            min_zoom: None,
            max_zoom: None,
        },
        // 56. marine_label_line_3
        StyleRule {
            source_layer: "marine_label".into(),
            style_type: StyleType::Symbol,
            color: p.marine_label,
            filter: Filter::All(vec![
                Filter::Eq("$type".into(), s("LineString")),
                Filter::Eq("labelrank".into(), n(3.0)),
            ]),
            min_zoom: None,
            max_zoom: None,
        },
        // 57. marine_label_point_3
        StyleRule {
            source_layer: "marine_label".into(),
            style_type: StyleType::Symbol,
            color: p.marine_label,
            filter: Filter::All(vec![
                Filter::Eq("$type".into(), s("Point")),
                Filter::Eq("labelrank".into(), n(3.0)),
            ]),
            min_zoom: None,
            max_zoom: None,
        },
        // 58. marine_label_line_2
        StyleRule {
            source_layer: "marine_label".into(),
            style_type: StyleType::Symbol,
            color: p.marine_label,
            filter: Filter::All(vec![
                Filter::Eq("$type".into(), s("LineString")),
                Filter::Eq("labelrank".into(), n(2.0)),
            ]),
            min_zoom: None,
            max_zoom: None,
        },
        // 59. marine_label_point_2
        StyleRule {
            source_layer: "marine_label".into(),
            style_type: StyleType::Symbol,
            color: p.marine_label,
            filter: Filter::All(vec![
                Filter::Eq("$type".into(), s("Point")),
                Filter::Eq("labelrank".into(), n(2.0)),
            ]),
            min_zoom: None,
            max_zoom: None,
        },
        // 60. marine_label_line_1
        StyleRule {
            source_layer: "marine_label".into(),
            style_type: StyleType::Symbol,
            color: p.marine_label,
            filter: Filter::All(vec![
                Filter::Eq("$type".into(), s("LineString")),
                Filter::Eq("labelrank".into(), n(1.0)),
            ]),
            min_zoom: None,
            max_zoom: None,
        },
        // 61. marine_label_point_1
        StyleRule {
            source_layer: "marine_label".into(),
            style_type: StyleType::Symbol,
            color: p.marine_label,
            filter: Filter::All(vec![
                Filter::Eq("$type".into(), s("Point")),
                Filter::Eq("labelrank".into(), n(1.0)),
            ]),
            min_zoom: None,
            max_zoom: None,
        },
        // 62. country_label_4
        StyleRule {
            source_layer: "country_label".into(),
            style_type: StyleType::Symbol,
            color: p.accent,
            filter: Filter::Gte("scalerank".into(), 4.0),
            min_zoom: None,
            max_zoom: None,
        },
        // 63. country_label_3
        StyleRule {
            source_layer: "country_label".into(),
            style_type: StyleType::Symbol,
            color: p.accent,
            filter: Filter::Eq("scalerank".into(), n(3.0)),
            min_zoom: None,
            max_zoom: None,
        },
        // 64. country_label_2
        StyleRule {
            source_layer: "country_label".into(),
            style_type: StyleType::Symbol,
            color: p.accent,
            filter: Filter::Eq("scalerank".into(), n(2.0)),
            min_zoom: None,
            max_zoom: None,
        },
        // 65. country_label_1
        StyleRule {
            source_layer: "country_label".into(),
            style_type: StyleType::Symbol,
            color: p.accent,
            filter: Filter::Eq("scalerank".into(), n(1.0)),
            min_zoom: None,
            max_zoom: None,
        },
    ]
}
