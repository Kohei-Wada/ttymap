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
- **When to emit a `UserIntent` vs a direct method call** — user intent goes through `App::dispatch`; internal data flow (frame arrival, widget polling) does not.
- **Controller split: by feature, not by domain** — if `App::dispatch` + cross-cutting helpers grow large.
- **Cleanup via `Drop`, not manual** — `RenderHandle`'s thread shutdown is handled by its Drop impl.
- **Frames are completed products** — main thread displays, does not compute.

## Architecture

The app uses a **three-thread model**:

1. **Main thread** (`src/app/`): Runs the event loop — drains completed frames from the render thread, polls plugins for async work, processes keyboard/mouse/resize events via crossterm, and asks ratatui to paint. State changes driven by user intent flow through `App::dispatch` (the single Receiver), which speaks the `UserIntent` vocabulary defined in `src/frontend/intent.rs`.

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
- **`src/frontend/`**: `Frontend` is the sole Receiver and owns the latest `MapFrame` directly (no `UiState` wrapper — built-in chrome lives in plugins now). `src/frontend/intent.rs` holds the `UserIntent` intent vocabulary (map-level actions nest under `UserIntent::Map(Action)` because `MapState` owns its own vocabulary; other variants sit at the top level); `src/frontend/event.rs` holds `AppEvent`, the unified queue payload (`Intent` / `FrameReady` / `Input` / `LuaIntent` / `Wake`); `src/frontend/mod.rs` holds `Frontend::dispatch` (the thin router) and cross-cutting methods like `apply_theme` / `handle_resize`. UI infrastructure lives under `frontend/`: `compositor/` (focus/modal stack), `palette/` (`:`-triggered picker), `ui.rs` (ratatui draw entry), `frame_timer.rs` (per-iteration wake source). Single side-effect boundary for user-intent state changes. See [docs/design.md](docs/design.md) for the UserIntent-vs-direct-API judgment rules.
- **`src/frontend/compositor/`**: Stack-based focus/modal system (helix-inspired). One primitive: a stack of `Component`s, where the top owns key focus. Components render side panels through `Component::render`, can paint world-space overlays via `Component::paint_on_map`, and can emit `UserIntent` via `Window`. Always-on chrome (info, scale_bar, attribution) is no longer a separate "overlay" stack — every Lua plugin's per-frame work runs through the unified `LuaTickRegistry::tick` (called from `ui::draw` immediately before `compositor.paint_on_map`). Submodules `map_api.rs` (drawing facade for both per-frame paths) and `layout.rs` (`PanelAnchor` vocabulary) live here too — both are crate-internal types consumed by the Lua bridge through `Component`. The earlier `src/plugin_api/` directory that hosted them was a misnamed bucket from the pre-Lua era; it's gone now that every plugin author goes through Lua.
- **`src/frontend/ui.rs`**: Single thin `draw()` entry — no state of its own. Lays out the map area + footer and routes through the compositor for overlays and panels.
- **`src/frontend/palette/`**: `:`-triggered command palette as an ephemeral `Component` (state per-open, discarded on pop). Provider sub-modes (theme picker, forward-geocode search) swap in place via `PaletteAction::SwitchProvider` or are pushed pre-loaded by their key activation. Providers can be sync (`OnEachKey` filter — command, theme) or async (`Debounced` filter, `poll()` to drain results, `is_loading()` for the spinner — search).
- **`src/commands/`**: CLI subcommands (currently `snap`). Each subcommand is one file with a `run()` entry point.
- **`src/input/`**: Input subsystem (peer of `map/` and `lua/`). `thread.rs` is the producer that blocks on `crossterm::event::read()` and pushes `AppEvent::Input`; `keymap.rs` holds the `KeyMap` table + `KeybindingOverrides` (read from `[keymap]` config and Lua `keymap.set`); `mouse.rs` holds the `MouseAdapter` that translates raw mouse events to `UserIntent`. Keyboard supports count prefixes for pan (e.g., `5j`); `:` opens the command palette, `/` opens the palette pre-loaded with the search provider.
- **`src/geo.rs`**: Web Mercator projection math — lon/lat ↔ tile coordinates, distance calculations.
- **`src/map/render/label.rs`**: Collision-free label placement buffer.
- **`src/lua/`**: Lua scripted plugins (mlua + Lua 5.4 vendored). All in-tree plugins live here — Rust-side `src/plugin/` is gone. nvim-style: any `.lua` under `<runtime>/plugin/` is a plugin, identified by file stem. Plugins join host loops by calling `ttymap.api.frame.on_tick(fn)` / `register_palette_command` / `register_keybind`. Layout: `src/lua/ttymap/` builds the `ttymap` global (Rust→Lua API binding); `src/lua/bridge/` adapts Lua specs to Rust traits (`Component`, `PaletteProvider`); top-level `mod.rs` / `registry.rs` / `runtimepath.rs` / `init_lua.rs` own discovery, the per-frame tick dispatcher, and the separate config-DSL state. See **[docs/lua-architecture.md](docs/lua-architecture.md)** for the full surface — every namespace, activation surfaces, drain pattern, runtime path resolution, config chain, bundled plugins. Migration guide: [docs/lua-plugin-migration.md](docs/lua-plugin-migration.md).

## Rust Edition

Uses Rust **2024 edition** — supports `let chains` in `if let` and `while let` natively (no feature flag needed).
