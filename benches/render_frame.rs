//! Render a single frame from a pre-decoded tile. Covers per-feature
//! filter eval, scale_ring, polygon clipping, polyline drawing, and
//! braille buffer updates.

use std::hint::black_box;
use std::sync::Arc;

use criterion::{Criterion, criterion_group, criterion_main};

use ttymap::color_palette::ThemeId;
use ttymap::map::render::renderer::{Renderer, TileData};
use ttymap::map::render::view::VisibleTile;
use ttymap::map::styler::Styler;
use ttymap::map::tile::decode;

const SAMPLE: &[u8] = include_bytes!("fixtures/z14.pbf");

/// Build a single-tile `TileData` matching the sample tile's coordinates,
/// positioned at the top-left of the canvas. The actual x/y/z values
/// don't matter for draw timing, only the geometry density does.
fn build_tile_data(decoded: ttymap::map::tile::decode::DecodedTile) -> Vec<TileData> {
    let vis = VisibleTile {
        x: 0,
        y: 0,
        z: 14,
        pos_x: 0.0,
        pos_y: 0.0,
        size: 256.0,
    };
    let layers = decoded
        .layers
        .into_iter()
        .map(|(name, tl)| {
            let feats = tl.tree.iter().cloned().collect::<Vec<_>>();
            (name, feats)
        })
        .collect();
    vec![TileData { vis, layers }]
}

fn bench_render_frame(c: &mut Criterion) {
    let decoded = decode::decode(SAMPLE);
    let tile_data = build_tile_data(decoded);
    let styler = Arc::new(Styler::new(ThemeId::Dark));
    // Typical terminal: 200×80 cells → 400×320 braille pixels.
    let mut renderer = Renderer::new(styler, "en".to_string(), 400, 320);

    c.bench_function("render_frame", |b| {
        b.iter(|| {
            let _ = renderer.draw(black_box(&tile_data), black_box(14.0));
        })
    });
}

criterion_group!(benches, bench_render_frame);
criterion_main!(benches);
