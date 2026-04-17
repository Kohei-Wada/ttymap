# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project

termap is a terminal-based map viewer written in Rust. It renders Mapbox Vector Tiles (MVT/protobuf) as Unicode Braille characters in the terminal, similar to mapscii. Default tile source is `http://mapscii.me/`.

## Build & Development

```bash
cargo build              # build (runs build.rs to compile proto/vector_tile.proto via protox)
cargo run                # run with defaults (Berlin, auto-zoom)
cargo run -- --lat 35.68 --lon 139.76 --zoom 10  # custom location
cargo run -- --style bright                        # alternate style
cargo test               # run all tests
cargo test test_name     # run a single test
cargo clippy             # lint
```

The build step compiles `proto/vector_tile.proto` using protox (no system protoc required). The generated Rust code is included at runtime via `include!(concat!(env!("OUT_DIR"), "/vector_tile.rs"))` in `src/tile.rs`.

## Architecture

The app uses a **three-thread model**:

1. **Main thread** (`app.rs`): Runs the event loop — processes keyboard/resize events via crossterm, manages map state (center, zoom), and writes completed frames to stdout. Uses 16ms poll timeout for responsive input.

2. **Render thread** (`render_thread.rs`): Receives `MapState` snapshots via `mpsc` channel, renders frames, and sends back completed frame strings. Communicates through `RenderHandle` with a busy flag (`AtomicBool`) — if the render thread is busy when a new draw is requested, the request is deferred (`pending_redraw`). Also polls for completed tile fetches every 50ms and re-renders when new tiles arrive.

3. **Tile fetch threads** (`tile_source.rs`): Each missing tile spawns a one-off thread for HTTP fetch. Completed tiles are sent back via `mpsc` channel to be polled by the render thread.

### Rendering pipeline

`MapState` → `Renderer::draw_state()` → visible tiles → spatial query (R-tree) → draw features by layer order (fills/lines first, then symbols sorted by priority) → `Canvas` → `BrailleBuffer::frame()` → ANSI escape string

Key modules:
- **`renderer.rs`**: Orchestrates tile fetching, spatial queries, and drawing. Determines visible tiles from center/zoom, queries each tile layer's R-tree for on-screen features, draws non-symbol features first then symbols sorted by `sort` key.
- **`tile.rs`**: Decodes protobuf MVT tiles into `DecodedTile` with per-layer R-trees (`rstar`) for spatial indexing. Applies style rules during decode.
- **`canvas.rs` / `braille.rs`**: 2×4 pixel Braille rendering. Each terminal cell maps to 8 sub-pixels. Supports polyline (with line width via Bresenham), polygon fill (via `earcutr` triangulation), and text overlay. Colors use ANSI 256-color palette.
- **`styler/`**: Defines map styles as Rust data structures with style presets (Dark/Bright). Each preset provides layer rules (filter expressions, color/width by zoom level). Applied during tile decode to produce styled `Feature` objects.
- **`tile_source.rs`**: Two-tier cache (LRU memory cache of 16 tiles + optional disk cache via `directories` crate). Background HTTP fetches with in-flight dedup.
- **`geo.rs`**: Web Mercator projection math — lon/lat ↔ tile coordinates, distance calculations.
- **`input.rs`**: Vim-style key handling with modes (Normal, Search `/`, Command `:`). Supports count prefixes for pan (e.g., `5j`).
- **`label.rs`**: Collision-free label placement buffer.
- **`layer.rs`**: Marker layer for A/B point distance measurement.

## Rust Edition

Uses Rust **2024 edition** — supports `let chains` in `if let` and `while let` natively (no feature flag needed).
