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

The build step compiles `proto/vector_tile.proto` using protox (no system protoc required). The generated Rust code is included at runtime via `include!(concat!(env!("OUT_DIR"), "/vector_tile.rs"))` in `src/map/tile/decode/mod.rs`.

## Design philosophy

See [docs/design.md](docs/design.md) for load-bearing design decisions:
- **When to emit an `AppMsg` vs a direct method call** — user intent goes through `App::dispatch`; internal data flow (frame arrival, widget polling) does not.
- **Controller split: by feature, not by domain** — if `App::dispatch` + cross-cutting helpers grow large.
- **Cleanup via `Drop`, not manual** — `RenderHandle`'s thread shutdown is handled by its Drop impl.
- **Frames are completed products** — main thread displays, does not compute.

## Architecture

The app uses a **three-thread model**:

1. **Main thread** (`src/app/`): Runs the event loop — drains completed frames from the render thread, polls plugins for async work, processes keyboard/mouse/resize events via crossterm, and asks ratatui to paint. State changes driven by user intent flow through `App::dispatch` (the single Receiver), which speaks the `AppMsg` vocabulary defined in `src/app/msg.rs`.

2. **Render thread** (`src/map/render/thread.rs`): Owns a `RenderPipeline` (tile cache + renderer). Receives `RenderTask` messages (`Draw(Viewport)` / `Resize` / `SetStyler` / `Shutdown`) via crossbeam-channel, and sends completed `MapFrame`s back. The loop is **purely event-driven**: a `crossbeam::select!` parks on the task channel and on a wake channel pinged by the decoder thread on each tile arrival — no timeout-based polling.

3. **Tile fetch + decode pipeline** (`src/map/tile/`): Three-layer flow `FetchLane → decoder → TileCache`. The fetch lane runs a fixed worker pool over a priority queue (`fetch/lane.rs`) and delegates per-tile bytes acquisition to a `TileFetcher` impl. Disk cache is a decorator (`fetch/disk_cached.rs`) that wraps the slow inner (today `HttpFetcher`) with read-through / write-through. A dedicated decoder thread (`decoder.rs`) parses MVT bytes off the render thread and forwards `DecodedTile`s to the cache via `mpsc`. The cache also keeps a synchronous **render-thread disk fast path** (`tile::disk` + `cache::DiskFastPath`) that bypasses the worker queue for already-on-disk tiles, which is the hot case during fast pan / zoom.

### Rendering pipeline

`Viewport` → `RenderPipeline::render()` → visible tiles → spatial query (R-tree) → draw features by layer order (fills/lines first, then symbols sorted by priority) → `Canvas` → `MapFrame` (grid of `MapCell { ch, fg, bg }`) → main thread paints via ratatui.

Key modules:
- **`src/map/render/renderer.rs`**: Orchestrates tile fetching, spatial queries, and drawing. Determines visible tiles from center/zoom, queries each tile layer's R-tree for on-screen features, draws non-symbol features first then symbols sorted by `sort` key.
- **`src/map/tile/decode/`**: Decodes protobuf MVT tiles into `DecodedTile` with per-layer R-trees (`rstar`) for spatial indexing. `mod.rs` owns the public types and the top-level `decode()` entry; `geometry.rs` is the zigzag + command-stream decoder; `tags.rs` decodes per-feature tag pairs; `decompress.rs` sniffs and unwraps gzip.
- **`src/map/render/canvas.rs` / `braille.rs`**: 2×4 pixel Braille rendering. Each terminal cell maps to 8 sub-pixels. Supports polyline (with line width via Bresenham), polygon fill (via `earcutr` triangulation), and text overlay. Colors use the xterm-256 palette.
- **`src/map/render/frame.rs`**: `MapFrame` — the completed grid of `MapCell { ch, fg, bg }` plus the view (center/zoom) it was rendered at, so overlays can project coordinates against the same frame regardless of staleness.
- **`src/map/styler/`**: Defines map styles as Rust data structures. `schema/mapscii.rs` is the single rule source (filter expressions, style_type, min/max zoom); themes vary only by `ColorPalette` swap. Applied during tile decode to produce styled `Feature` objects. Future schemas (Protomaps etc.) land as `schema/<name>.rs`.
- **`src/theme/`**: Colour data (`palette.rs` — `ColorPalette` + `DARK` / `BRIGHT` consts, xterm-256 indices) plus the ratatui adapter (`ui.rs` — `UiTheme`). `ThemeId` lives in `theme/mod.rs` and is the single source of truth for "which theme". Styler consumes `ColorPalette`; UI code consumes `UiTheme`.
- **`src/map/tile/cache.rs`**: Orchestrator — LRU memory cache (`lru` crate), view state (center / zoom), prefetch ring, and the channel drain (`poll_completed`). On a memory miss it consults the optional `DiskFastPath` (synchronous disk read + push to decoder, bypassing the worker queue) before enqueueing for the slow lane.
- **`src/map/tile/disk.rs`**: Free-function disk read/write helpers used by both `fetch::DiskCachedFetcher` (worker-side, read+write through) and `cache::TileCache` (render-thread fast path, read-only). Layout: `{cache_dir}/{z}/{x}-{y}.pbf`.
- **`src/map/tile/fetch/`**: `TileFetcher` (per-backend trait — "key → bytes"), the generic `FetchLane<F>` (queue / workers / dedup / priority), and the `DiskCachedFetcher<F>` decorator that adds a disk read-through / write-through layer to any inner fetcher. `http.rs` is the only inner backend today; new backends (mbtiles, pmtiles, …) add a `TileFetcher` impl + a branch in `app::build_tile_cache`.
- **`src/map/tile/decoder.rs`**: Single-thread relay that reads bytes from the fetch lane, calls `decode::decode`, and forwards `DecodedTile`s to the cache. Empty bytes (negative cache from failed fetches) bypass `decode()` and surface as `DecodedTile::empty()`.
- **`src/app/`**: `App` is the single Receiver and owns the latest `MapFrame` directly (no `UiState` wrapper — built-in chrome lives in plugins now). `src/app/msg.rs` holds the `AppMsg` intent vocabulary (map-level actions nest under `AppMsg::Map(Action)` because `MapState` owns its own vocabulary; other variants sit at the top level); `src/app/mod.rs` holds `App::dispatch` (the thin router) and cross-cutting methods like `apply_theme` / `handle_resize`. Mouse handling lives in `src/app/mouse.rs`. Single side-effect boundary for user-intent state changes. See [docs/design.md](docs/design.md) for the AppMsg-vs-direct-API judgment rules.
- **`src/compositor/`**: Stack-based focus/modal system (helix-inspired). One primitive: a stack of `Component`s, where the top owns key focus. Replaced the old `FocusManager` + `Plugin` trilogy. Components render map overlays through `Component::paint_on_map`, side panels through `Component::render`, and can emit `AppMsg` via `Window`. Always-on chrome (info, scale_bar, attribution) registers as `overlays` — non-focusable, painted every frame.
- **`src/ui.rs`**: Single thin `draw()` entry — no state of its own. Lays out the map area + footer and routes through the compositor for overlays and panels.
- **`src/palette/`**: `:`-triggered command palette as an ephemeral `Component` (state per-open, discarded on pop). Provider sub-modes (theme picker, forward-geocode search) swap in place via `PaletteAction::SwitchProvider` or are pushed pre-loaded by their key activation. Providers can be sync (`OnEachKey` filter — command, theme) or async (`Debounced` filter, `poll()` to drain results, `is_loading()` for the spinner — search).
- **`src/plugin_api/`**: The plugin SDK — `MapApi`, `Window`, `RenderWindow`, `LayoutConfig`, `ListPanel`, `PolledFeed`, `InitialJump`, `Throttle`, `AsyncJob`, `NominatimClient`. Plugins import via `crate::plugin_api::prelude::*`.
- **`src/widget/`**: Neutral widget-descriptor vocabulary (`Paragraph`, `List`, `Table`, …). Plugins build descriptors; `RenderWindow` does the `From<widget::X> for ratatui::X` conversion internally. Keeps `ratatui::*` out of plugin code.
- **`src/commands/`**: CLI subcommands (currently `snap`). Each subcommand is one file with a `run()` entry point.
- **`src/keymap.rs`**: Keybinding table + `KeybindingOverrides` (read from `[keymap]` config). Keyboard supports count prefixes for pan (e.g., `5j`); `:` opens the command palette, `/` opens the palette pre-loaded with the search provider.
- **`src/geo.rs`**: Web Mercator projection math — lon/lat ↔ tile coordinates, distance calculations.
- **`src/map/render/label.rs`**: Collision-free label placement buffer.
- **`src/plugin/`**: Plugins (aircraft, attribution, export, help, here, info, iss, quake, scalebar, search, wiki) — composable UI panels with async work via `poll()`. Built-in chrome (info / scalebar / attribution) is structured as plugins too. Search is the unusual one: instead of being a `Component`, it's a `PaletteProvider` (in `src/plugin/search/mod.rs`) — its `register()` binds `/` to push the palette pre-loaded with the provider.
- **`src/lua/`**: Lua scripted plugins (mlua + Lua 5.4 vendored). `LuaComponent` (`component.rs`) implements `Component` by dispatching to a Lua module table; `register()` (in `mod.rs`) wires bundled scripts in `scripts/*.lua` into the registrar. **Opt-in only** — `app::build_registrar` calls it solely when `[lua] enabled = true` is set in config, because today's bundled scripts (`hello.lua`) are demos, not features. See `docs/lua-bridge-surface.md` for the bridge surface scope.

## Rust Edition

Uses Rust **2024 edition** — supports `let chains` in `if let` and `while let` natively (no feature flag needed).
