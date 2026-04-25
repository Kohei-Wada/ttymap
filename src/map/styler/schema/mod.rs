//! Per-schema rule tables.
//!
//! A schema (`mapscii`, future `protomaps`, …) defines what
//! `source_layer` names and `class` values exist in the tiles a given
//! tile backend produces. Each schema lives in its own module and
//! exposes a single `rules(palette) -> Vec<StyleRule>` function. The
//! palette swap is the **theme** axis (dark / bright / …); the rule
//! shape is owned by the schema.
//!
//! No `Schema` enum on purpose (#46 wontfix) — schema is named via
//! the constructor that called it (today `Styler::new` always uses
//! mapscii). When a second schema lands, add `Styler::for_protomaps`
//! and let the tile backend factory pick which constructor to call.

pub mod mapscii;
