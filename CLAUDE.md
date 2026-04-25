# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project

ttymap is a terminal-based map viewer written in Rust. It renders Mapbox Vector Tiles (MVT/protobuf) as Unicode Braille characters in the terminal, similar to mapscii. Default tile source is `http://mapscii.me/`.

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

The build step compiles `proto/vector_tile.proto` using protox (no system protoc required). The generated Rust code is included at runtime via `include!(concat!(env!("OUT_DIR"), "/vector_tile.rs"))` in `src/map/tile/decode.rs`.

## Design philosophy

See [docs/design.md](docs/design.md) for load-bearing design decisions:
- **When to emit an `AppMsg` vs a direct method call** — user intent goes through `App::dispatch`; internal data flow (frame arrival, widget polling) does not.
- **Controller split: by feature, not by domain** — if `App::dispatch` + cross-cutting helpers grow large.
- **Cleanup via `Drop`, not manual** — `RenderHandle`'s thread shutdown is handled by its Drop impl.
- **Frames are completed products** — main thread displays, does not compute.

## Architecture

The app uses a **three-thread model**:

1. **Main thread** (`src/app/`): Runs the event loop — drains completed frames from the render thread, polls plugins for async work, processes keyboard/mouse/resize events via crossterm, and asks ratatui to paint. State changes driven by user intent flow through `App::dispatch` (the single Receiver), which speaks the `AppMsg` vocabulary defined in `src/app/msg.rs`.

2. **Render thread** (`src/map/render/thread.rs`): Owns a `RenderPipeline` (tile cache + renderer). Receives `RenderTask` messages (`Draw(Viewport)` / `Resize` / `SetStyler` / `Shutdown`) via `mpsc`, and sends completed `MapFrame`s back. Also polls the tile cache for completed fetches and re-renders when new tiles arrive.

3. **Tile fetch threads** (`src/map/tile/fetch/`): Each missing tile spawns a short-lived thread for HTTP fetch. Completed bytes are decoded into `DecodedTile` and delivered via `mpsc` for the render thread to pick up.

### Rendering pipeline

`Viewport` → `RenderPipeline::render()` → visible tiles → spatial query (R-tree) → draw features by layer order (fills/lines first, then symbols sorted by priority) → `Canvas` → `MapFrame` (grid of `MapCell { ch, fg, bg }`) → main thread paints via ratatui.

Key modules:
- **`src/map/render/renderer.rs`**: Orchestrates tile fetching, spatial queries, and drawing. Determines visible tiles from center/zoom, queries each tile layer's R-tree for on-screen features, draws non-symbol features first then symbols sorted by `sort` key.
- **`src/map/tile/decode.rs`**: Decodes protobuf MVT tiles into `DecodedTile` with per-layer R-trees (`rstar`) for spatial indexing. Applies style rules during decode.
- **`src/map/render/canvas.rs` / `braille.rs`**: 2×4 pixel Braille rendering. Each terminal cell maps to 8 sub-pixels. Supports polyline (with line width via Bresenham), polygon fill (via `earcutr` triangulation), and text overlay. Colors use the xterm-256 palette.
- **`src/map/render/frame.rs`**: `MapFrame` — the completed grid of `MapCell { ch, fg, bg }` plus the view (center/zoom) it was rendered at, so overlays can project coordinates against the same frame regardless of staleness.
- **`src/map/styler/`**: Defines map styles as Rust data structures. `schema/mapscii.rs` is the single rule source (filter expressions, style_type, min/max zoom); themes vary only by `ColorPalette` swap. Applied during tile decode to produce styled `Feature` objects. Future schemas (Protomaps etc.) land as `schema/<name>.rs`.
- **`src/theme/`**: Colour data (`palette.rs` — `ColorPalette` + `DARK` / `BRIGHT` consts, xterm-256 indices) plus the ratatui adapter (`ui.rs` — `UiTheme`). `ThemeId` lives in `theme/mod.rs` and is the single source of truth for "which theme". Styler consumes `ColorPalette`; UI code consumes `UiTheme`.
- **`src/map/tile/cache.rs` / `src/map/tile/fetch/`**: Two-tier cache (LRU memory cache + optional on-disk cache via `directories` crate) and pluggable HTTP clients, with in-flight dedup.
- **`src/app/`**: `App` is the single Receiver and owns the latest `MapFrame` directly (no `UiState` wrapper — built-in chrome lives in plugins now). `src/app/msg.rs` holds the `AppMsg` intent vocabulary (map-level actions nest under `AppMsg::Map(Action)` because `MapState` owns its own vocabulary; other variants sit at the top level); `src/app/mod.rs` holds `App::dispatch` (the thin router) and cross-cutting methods like `apply_theme` / `handle_resize`. Mouse handling lives in `src/app/mouse.rs`. Single side-effect boundary for user-intent state changes. See [docs/design.md](docs/design.md) for the AppMsg-vs-direct-API judgment rules.
- **`src/compositor/`**: Stack-based focus/modal system (helix-inspired). One primitive: a stack of `Component`s, where the top owns key focus. Replaced the old `FocusManager` + `Plugin` trilogy. Components render map overlays through `Component::paint_on_map`, side panels through `Component::render`, and can emit `AppMsg` via `Window`. Always-on chrome (info, scale_bar, attribution) registers as `overlays` — non-focusable, painted every frame.
- **`src/ui.rs`**: Single thin `draw()` entry — no state of its own. Lays out the map area + footer and routes through the compositor for overlays and panels.
- **`src/palette/`**: `:`-triggered command palette as an ephemeral `Component` (state per-open, discarded on pop). Provider sub-modes (e.g. theme picker) swap in place via `PaletteAction::SwitchProvider`.
- **`src/plugin_api/`**: The plugin SDK — `MapApi`, `Window`, `RenderWindow`, `LayoutConfig`, `ListPanel`, `PolledFeed`, `Throttle`, `AsyncJob`, `NominatimClient`. Plugins import via `crate::plugin_api::prelude::*`.
- **`src/widget/`**: Neutral widget-descriptor vocabulary (`Paragraph`, `List`, `Table`, …). Plugins build descriptors; `RenderWindow` does the `From<widget::X> for ratatui::X` conversion internally. Keeps `ratatui::*` out of plugin code.
- **`src/commands/`**: CLI subcommands (currently `snap`). Each subcommand is one file with a `run()` entry point.
- **`src/keymap.rs`**: Keybinding table + `KeybindingOverrides` (read from `[keymap]` config). Keyboard supports count prefixes for pan (e.g., `5j`); `:` opens the command palette, `/` opens the search plugin.
- **`src/geo.rs`**: Web Mercator projection math — lon/lat ↔ tile coordinates, distance calculations.
- **`src/map/render/label.rs`**: Collision-free label placement buffer.
- **`src/plugin/`**: Plugins (aircraft, attribution, export, help, here, info, iss, quake, scalebar, search, wiki) — composable UI panels with async work via `poll()`. Built-in chrome (info / scalebar / attribution) is structured as plugins too.

## Rust Edition

Uses Rust **2024 edition** — supports `let chains` in `if let` and `while let` natively (no feature flag needed).
