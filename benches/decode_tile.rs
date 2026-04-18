//! Decode a fixed MVT protobuf tile. Covers the whole decode pipeline:
//! gzip decompression, protobuf parsing, tag interning, geometry decode,
//! and R-tree bulk-load.

use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};

use ttymap::tile::decode;

/// Sample tile fetched once from mapscii.me (z14 over Tokyo). About 5 KB
/// gzipped, ~11 KB raw, ~1000 features across ~15 layers.
const SAMPLE: &[u8] = include_bytes!("fixtures/z14.pbf");

fn bench_decode_tile(c: &mut Criterion) {
    c.bench_function("decode_tile", |b| {
        b.iter(|| decode::decode(black_box(SAMPLE)))
    });
}

criterion_group!(benches, bench_decode_tile);
criterion_main!(benches);
