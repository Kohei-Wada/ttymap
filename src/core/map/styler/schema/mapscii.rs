//! Mapscii MVT schema rules — single source of truth, theme-agnostic.
//!
//! Replaces the old `preset_dark.rs` / `preset_bright.rs` parallel
//! files. Both themes consume the same `Vec<StyleRule>`; theme
//! variation comes through `ColorPalette` only. See `docs/design.md`
//! ("Schema-theme orthogonality") for the rationale.

use crate::theme::ColorPalette;

use super::super::filter::Filter;
use super::super::{StyleRule, StyleType};
use crate::core::map::tile::PropertyValue as PV;

fn s(v: &str) -> PV {
    PV::String(std::sync::Arc::from(v))
}

fn n(v: f64) -> PV {
    PV::Number(v)
}

pub fn rules(p: &ColorPalette) -> Vec<StyleRule> {
    vec![
        // landuse_overlay_national_park
        StyleRule {
            source_layer: "landuse_overlay".into(),
            style_type: StyleType::Fill,
            color: p.landuse_overlay,
            filter: Filter::Eq("class".into(), s("national_park")),
            min_zoom: None,
            max_zoom: None,
        },
        // landuse_park
        StyleRule {
            source_layer: "landuse".into(),
            style_type: StyleType::Fill,
            color: p.landuse_park,
            filter: Filter::Eq("class".into(), s("park")),
            min_zoom: None,
            max_zoom: None,
        },
        // landuse_cemetery
        StyleRule {
            source_layer: "landuse".into(),
            style_type: StyleType::Line,
            color: p.landuse_cemetery,
            filter: Filter::Eq("class".into(), s("cemetery")),
            min_zoom: None,
            max_zoom: None,
        },
        // landuse_hospital
        StyleRule {
            source_layer: "landuse".into(),
            style_type: StyleType::Line,
            color: p.landuse_hospital,
            filter: Filter::Eq("class".into(), s("hospital")),
            min_zoom: None,
            max_zoom: None,
        },
        // landuse_school
        StyleRule {
            source_layer: "landuse".into(),
            style_type: StyleType::Line,
            color: p.landuse_school,
            filter: Filter::Eq("class".into(), s("school")),
            min_zoom: None,
            max_zoom: None,
        },
        // landuse_wood
        StyleRule {
            source_layer: "landuse".into(),
            style_type: StyleType::Line,
            color: p.landuse_wood,
            filter: Filter::Eq("class".into(), s("wood")),
            min_zoom: None,
            max_zoom: None,
        },
        // waterway (everything except river/stream/canal)
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
        // waterway_river
        StyleRule {
            source_layer: "waterway".into(),
            style_type: StyleType::Line,
            color: p.waterway_deep,
            filter: Filter::Eq("class".into(), s("river")),
            min_zoom: None,
            max_zoom: None,
        },
        // waterway_stream_canal
        StyleRule {
            source_layer: "waterway".into(),
            style_type: StyleType::Line,
            color: p.waterway,
            filter: Filter::In("class".into(), vec![s("stream"), s("canal")]),
            min_zoom: None,
            max_zoom: None,
        },
        // water
        StyleRule {
            source_layer: "water".into(),
            style_type: StyleType::Fill,
            color: p.water,
            filter: Filter::Always,
            min_zoom: None,
            max_zoom: None,
        },
        // aeroway_fill
        StyleRule {
            source_layer: "aeroway".into(),
            style_type: StyleType::Fill,
            color: p.aeroway,
            filter: Filter::Eq("$type".into(), s("Polygon")),
            min_zoom: Some(11.0),
            max_zoom: None,
        },
        // aeroway_runway
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
        // aeroway_taxiway
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
        // building
        StyleRule {
            source_layer: "building".into(),
            style_type: StyleType::Line,
            color: p.building,
            filter: Filter::Always,
            min_zoom: Some(14.5),
            max_zoom: None,
        },
        // tunnel_path_pedestrian
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
        // tunnel_motorway_link
        StyleRule {
            source_layer: "road".into(),
            style_type: StyleType::Line,
            color: p.road_motorway,
            filter: Filter::All(vec![
                Filter::Eq("structure".into(), s("tunnel")),
                Filter::Eq("class".into(), s("motorway_link")),
            ]),
            min_zoom: None,
            max_zoom: None,
        },
        // tunnel_service_track
        StyleRule {
            source_layer: "road".into(),
            style_type: StyleType::Line,
            color: p.road_street,
            filter: Filter::All(vec![
                Filter::Eq("structure".into(), s("tunnel")),
                Filter::In("class".into(), vec![s("service"), s("track")]),
            ]),
            min_zoom: None,
            max_zoom: None,
        },
        // tunnel_link
        StyleRule {
            source_layer: "road".into(),
            style_type: StyleType::Line,
            color: p.tunnel_link,
            filter: Filter::All(vec![
                Filter::Eq("structure".into(), s("tunnel")),
                Filter::Eq("class".into(), s("link")),
            ]),
            min_zoom: None,
            max_zoom: None,
        },
        // tunnel_street
        StyleRule {
            source_layer: "road".into(),
            style_type: StyleType::Line,
            color: p.road_street,
            filter: Filter::All(vec![
                Filter::Eq("structure".into(), s("tunnel")),
                Filter::In("class".into(), vec![s("street"), s("street_limited")]),
            ]),
            min_zoom: None,
            max_zoom: None,
        },
        // tunnel_secondary_tertiary
        StyleRule {
            source_layer: "road".into(),
            style_type: StyleType::Line,
            color: p.tunnel_link,
            filter: Filter::All(vec![
                Filter::Eq("structure".into(), s("tunnel")),
                Filter::In("class".into(), vec![s("secondary"), s("tertiary")]),
            ]),
            min_zoom: None,
            max_zoom: None,
        },
        // tunnel_trunk_primary
        StyleRule {
            source_layer: "road".into(),
            style_type: StyleType::Line,
            color: p.tunnel_link,
            filter: Filter::All(vec![
                Filter::Eq("structure".into(), s("tunnel")),
                Filter::In("class".into(), vec![s("trunk"), s("primary")]),
            ]),
            min_zoom: None,
            max_zoom: None,
        },
        // tunnel_motorway
        StyleRule {
            source_layer: "road".into(),
            style_type: StyleType::Line,
            color: p.tunnel_motorway,
            filter: Filter::All(vec![
                Filter::Eq("structure".into(), s("tunnel")),
                Filter::Eq("class".into(), s("motorway")),
            ]),
            min_zoom: None,
            max_zoom: None,
        },
        // tunnel_major_rail
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
        // tunnel_major_rail_hatching
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
        // road_path_pedestrian
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
        // road_motorway_link
        StyleRule {
            source_layer: "road".into(),
            style_type: StyleType::Line,
            color: p.road_motorway,
            filter: Filter::All(vec![
                Filter::Eq("class".into(), s("motorway_link")),
                Filter::NotIn("structure".into(), vec![s("bridge"), s("tunnel")]),
            ]),
            min_zoom: Some(12.0),
            max_zoom: None,
        },
        // road_service_track
        StyleRule {
            source_layer: "road".into(),
            style_type: StyleType::Line,
            color: p.road_street,
            filter: Filter::All(vec![
                Filter::In("class".into(), vec![s("service"), s("track")]),
                Filter::NotIn("structure".into(), vec![s("bridge"), s("tunnel")]),
            ]),
            min_zoom: None,
            max_zoom: None,
        },
        // road_link
        StyleRule {
            source_layer: "road".into(),
            style_type: StyleType::Line,
            color: p.road_trunk_primary,
            filter: Filter::All(vec![
                Filter::Eq("class".into(), s("link")),
                Filter::NotIn("structure".into(), vec![s("bridge"), s("tunnel")]),
            ]),
            min_zoom: Some(13.0),
            max_zoom: None,
        },
        // road_street
        StyleRule {
            source_layer: "road".into(),
            style_type: StyleType::Line,
            color: p.road_street,
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
        // road_secondary_tertiary
        StyleRule {
            source_layer: "road".into(),
            style_type: StyleType::Line,
            color: p.road_trunk_primary,
            filter: Filter::All(vec![
                Filter::In("class".into(), vec![s("secondary"), s("tertiary")]),
                Filter::NotIn("structure".into(), vec![s("bridge"), s("tunnel")]),
            ]),
            min_zoom: None,
            max_zoom: None,
        },
        // road_trunk_primary
        StyleRule {
            source_layer: "road".into(),
            style_type: StyleType::Line,
            color: p.road_trunk_primary,
            filter: Filter::All(vec![
                Filter::In("class".into(), vec![s("trunk"), s("primary")]),
                Filter::NotIn("structure".into(), vec![s("bridge"), s("tunnel")]),
            ]),
            min_zoom: None,
            max_zoom: None,
        },
        // road_motorway
        StyleRule {
            source_layer: "road".into(),
            style_type: StyleType::Line,
            color: p.road_motorway,
            filter: Filter::All(vec![
                Filter::Eq("class".into(), s("motorway")),
                Filter::NotIn("structure".into(), vec![s("bridge"), s("tunnel")]),
            ]),
            min_zoom: Some(5.0),
            max_zoom: None,
        },
        // road_major_rail
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
        // road_major_rail_hatching
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
        // bridge_path_pedestrian
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
        // bridge_motorway_link
        StyleRule {
            source_layer: "road".into(),
            style_type: StyleType::Line,
            color: p.road_motorway,
            filter: Filter::All(vec![
                Filter::Eq("structure".into(), s("bridge")),
                Filter::Eq("class".into(), s("motorway_link")),
            ]),
            min_zoom: None,
            max_zoom: None,
        },
        // bridge_service_track
        StyleRule {
            source_layer: "road".into(),
            style_type: StyleType::Line,
            color: p.road_street,
            filter: Filter::All(vec![
                Filter::Eq("structure".into(), s("bridge")),
                Filter::In("class".into(), vec![s("service"), s("track")]),
            ]),
            min_zoom: None,
            max_zoom: None,
        },
        // bridge_link
        StyleRule {
            source_layer: "road".into(),
            style_type: StyleType::Line,
            color: p.road_trunk_primary,
            filter: Filter::All(vec![
                Filter::Eq("structure".into(), s("bridge")),
                Filter::Eq("class".into(), s("link")),
            ]),
            min_zoom: None,
            max_zoom: None,
        },
        // bridge_street
        StyleRule {
            source_layer: "road".into(),
            style_type: StyleType::Line,
            color: p.road_street,
            filter: Filter::All(vec![
                Filter::Eq("structure".into(), s("bridge")),
                Filter::In("class".into(), vec![s("street"), s("street_limited")]),
            ]),
            min_zoom: None,
            max_zoom: None,
        },
        // bridge_secondary_tertiary
        StyleRule {
            source_layer: "road".into(),
            style_type: StyleType::Line,
            color: p.road_trunk_primary,
            filter: Filter::All(vec![
                Filter::Eq("structure".into(), s("bridge")),
                Filter::In("class".into(), vec![s("secondary"), s("tertiary")]),
            ]),
            min_zoom: None,
            max_zoom: None,
        },
        // bridge_trunk_primary
        StyleRule {
            source_layer: "road".into(),
            style_type: StyleType::Line,
            color: p.road_trunk_primary,
            filter: Filter::All(vec![
                Filter::Eq("structure".into(), s("bridge")),
                Filter::In("class".into(), vec![s("trunk"), s("primary")]),
            ]),
            min_zoom: None,
            max_zoom: None,
        },
        // bridge_motorway
        StyleRule {
            source_layer: "road".into(),
            style_type: StyleType::Line,
            color: p.road_motorway,
            filter: Filter::All(vec![
                Filter::Eq("structure".into(), s("bridge")),
                Filter::Eq("class".into(), s("motorway")),
            ]),
            min_zoom: None,
            max_zoom: None,
        },
        // bridge_major_rail
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
        // bridge_major_rail_hatching
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
        // admin_level_4 (admin_level >= 4, non-maritime)
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
        // admin_level_3 (admin_level == 3, non-maritime)
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
        // admin_level_2 (national, non-disputed, non-maritime)
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
        // admin_level_2_disputed
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
        // admin_level_3_maritime (admin_level >= 3, maritime)
        StyleRule {
            source_layer: "admin".into(),
            style_type: StyleType::Line,
            color: p.admin_maritime_3,
            filter: Filter::All(vec![
                Filter::Gte("admin_level".into(), 3.0),
                Filter::Eq("maritime".into(), n(1.0)),
            ]),
            min_zoom: None,
            max_zoom: None,
        },
        // admin_level_2_maritime
        StyleRule {
            source_layer: "admin".into(),
            style_type: StyleType::Line,
            color: p.admin_maritime_2,
            filter: Filter::All(vec![
                Filter::Eq("admin_level".into(), n(2.0)),
                Filter::Eq("maritime".into(), n(1.0)),
            ]),
            min_zoom: None,
            max_zoom: None,
        },
        // water_label
        StyleRule {
            source_layer: "water_label".into(),
            style_type: StyleType::Symbol,
            color: p.water_label,
            filter: Filter::Eq("$type".into(), s("Point")),
            min_zoom: None,
            max_zoom: None,
        },
        // poi_label_4
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
        // poi_label_3
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
        // poi_label_2
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
        // rail_station_label
        StyleRule {
            source_layer: "rail_station_label".into(),
            style_type: StyleType::Symbol,
            color: p.rail_station_label,
            filter: Filter::Always,
            min_zoom: None,
            max_zoom: None,
        },
        // poi_label_1
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
        // airport_label
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
        // road_label
        StyleRule {
            source_layer: "road_label".into(),
            style_type: StyleType::Symbol,
            color: p.road_label,
            filter: Filter::NotEq("class".into(), s("ferry")),
            min_zoom: Some(15.5),
            max_zoom: None,
        },
        // housenum_label
        StyleRule {
            source_layer: "housenum_label".into(),
            style_type: StyleType::Symbol,
            color: p.housenum_label,
            filter: Filter::Always,
            min_zoom: Some(16.5),
            max_zoom: None,
        },
        // place_label_other (hamlet/suburb/neighbourhood/island/islet)
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
        // place_label_village
        StyleRule {
            source_layer: "place_label".into(),
            style_type: StyleType::Symbol,
            color: p.place_village,
            filter: Filter::Eq("type".into(), s("village")),
            min_zoom: None,
            max_zoom: None,
        },
        // place_label_town
        StyleRule {
            source_layer: "place_label".into(),
            style_type: StyleType::Symbol,
            color: p.place_town,
            filter: Filter::Eq("type".into(), s("town")),
            min_zoom: None,
            max_zoom: None,
        },
        // place_label_city
        StyleRule {
            source_layer: "place_label".into(),
            style_type: StyleType::Symbol,
            color: p.place_city,
            filter: Filter::Eq("type".into(), s("city")),
            min_zoom: None,
            max_zoom: None,
        },
        // marine_label_line_4 (labelrank >= 4)
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
        // marine_label_4
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
        // marine_label_line_3
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
        // marine_label_point_3
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
        // marine_label_line_2
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
        // marine_label_point_2
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
        // marine_label_line_1
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
        // marine_label_point_1
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
        // country_label_4 (scalerank >= 4)
        StyleRule {
            source_layer: "country_label".into(),
            style_type: StyleType::Symbol,
            color: p.accent,
            filter: Filter::Gte("scalerank".into(), 4.0),
            min_zoom: None,
            max_zoom: None,
        },
        // country_label_3
        StyleRule {
            source_layer: "country_label".into(),
            style_type: StyleType::Symbol,
            color: p.accent,
            filter: Filter::Eq("scalerank".into(), n(3.0)),
            min_zoom: None,
            max_zoom: None,
        },
        // country_label_2
        StyleRule {
            source_layer: "country_label".into(),
            style_type: StyleType::Symbol,
            color: p.accent,
            filter: Filter::Eq("scalerank".into(), n(2.0)),
            min_zoom: None,
            max_zoom: None,
        },
        // country_label_1
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
