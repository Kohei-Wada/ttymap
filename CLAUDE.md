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

The build step compiles `ttymap-engine/proto/vector_tile.proto` using protox (no system protoc required). The generated Rust code is included at runtime via `include!(concat!(env!("OUT_DIR"), "/vector_tile.rs"))` in `ttymap-engine/src/map/tile/decode/mod.rs`.

## Workspace layout

The repository is a two-crate Cargo workspace:

- `ttymap-engine/` (`ttymap-engine`) — headless rendering engine. Owns the
  map subsystem (tile fetch + decode + cache, render thread, styler),
  the `MapFrame` produced for display, the color-palette data, the
  `geo` projection module, and the User-Agent-tagged HTTP client. **No
  ratatui or crossterm dependency** — the engine is consumable on its
  own (e.g. by the `snap` subcommand) without bringing in a TUI
  framework. Communication out is a `FrameSink` callback (`Box<dyn
  FnMut(MapFrame) -> bool + Send>`) so the engine never names the
  binary's `AppEvent` bus.
- `ttymap-tui/` (`ttymap-tui`) — TUI binary. Owns the App event loop,
  ratatui draw entry, compositor / palette / input / lua bridge, plus
  the ratatui-side theme adapter (`UiTheme`, `StyleKind`). Wraps
  `ttymap_engine::Config` with binary-only knobs (`geoip`, `runtime`,
  `plugins`, keybinding overrides) — engine-side fields are reached
  via `config.engine.<sub>.<field>`. The crate is named `ttymap-tui`
  but the produced executable is still `ttymap` (set via
  `[[bin]] name = "ttymap"`), so `cargo install` and
  `~/.cargo/bin/ttymap` are unchanged.

## Design philosophy

See [docs/design.md](docs/design.md) for load-bearing design decisions:
- **When to emit a `UserCommand` vs a direct method call** — user intent goes through `App::dispatch`; internal data flow (frame arrival, widget polling) does not.
- **Controller split: by feature, not by domain** — if `App::dispatch` + cross-cutting helpers grow large.
- **Cleanup via `Drop`, not manual** — `RenderHandle`'s thread shutdown is handled by its Drop impl.
- **Frames are completed products** — main thread displays, does not compute.

For the full system architecture (src tree, layering, message + render flow, focus model, concurrency) see [docs/architecture.md](docs/architecture.md). The summary below is enough to navigate the code; details belong in that doc.

## Architecture

### Source tree

The binary is **flat by feature**, Neovim-inspired. We tried a strict
`core/front` split (issue #212 Phase 4) and reverted it — it forced
too many exceptional placements (sidebar policy is "UI" but lived in
core because dispatcher owned it; theme_id leaked into core because
every command tracked it; etc.). The engine/binary boundary is a
genuine layering boundary so it lives at the **crate** level instead.

```
ttymap-engine/                (ttymap-engine — ratatui-free)
  src/
    config.rs                 engine-side settings (cache / map / render)
    geo.rs                    Web Mercator projection math
    map/                      tile + render + styler + viewport state
    shared/http/              User-Agent-tagged reqwest wrapper
    theme/                    palette data (ColorPalette + DARK/BRIGHT + ThemeId)
  proto/vector_tile.proto
  build.rs                    compiles MVT proto via protox
  benches/                    decode_tile / render_frame / tile_disk_hit

ttymap-tui/                   (ttymap-tui — ratatui + crossterm shell)
  src/
    command.rs                UserCommand vocabulary
    config.rs                 wraps ttymap_engine::Config (+ geoip/runtime/plugins)
    logging.rs                XDG state log
    app/                      event loop + dispatcher + ratatui draw entry
      mod.rs / dispatcher.rs / event.rs / frame_timer.rs / frame_widget.rs / ui.rs
    cli/                      CLI subcommands (snap)
    compositor/               focus stack + Component trait + Op + render
    input/                    keymap + mouse adapter + input thread
    palette/                  `:`-triggered picker UI
    theme/                    ratatui adapter — UiTheme + StyleKind
                              (re-exports ColorPalette/ThemeId/DARK/BRIGHT
                               from the engine)
    lua/                      plugin runtime — bridges binary (Component,
                              palette) and engine (MapApi, http)
    shared/geoip.rs           IP → lon/lat resolution (binary-only)
  runtime/                    bundled Lua plugins + init.lua scaffolding
```

The single layering rule: **the engine crate does not depend on
ratatui or crossterm**. The binary owns the event loop and all UI
adapters; the engine produces `MapFrame`s and is driven by a
`FrameSink` callback. Inside the binary, modules are flat peers
named for what they do.

### Three-thread model

1. **Main thread** (`ttymap-tui/src/app/`): Runs the event loop — drains completed frames from the render thread (delivered via the `FrameSink` callback the binary hands the engine at startup), polls plugins for async work, processes keyboard/mouse/resize events via crossterm, and asks ratatui to paint. State changes flow through `app::Dispatcher` (the single GoF Receiver, owned by `App`), which speaks the `UserCommand` vocabulary defined at the crate root in `ttymap-tui/src/command.rs` (placed there so every emission site reaches it via `crate::UserCommand` without depending upward on `app/`).

2. **Render thread** (`ttymap-engine/src/map/render/thread.rs`): Owns a `RenderPipeline` (tile cache + renderer). Receives `RenderTask` messages (`Draw { viewport, overlays }` / `Resize` / `SetStyler` / `Shutdown`) via crossbeam-channel, and sends completed `MapFrame`s back. `Draw` carries a `Vec<UserPolyline>` overlay batch drained from the App after each `ui::draw` so Lua-plugin polylines render in the same pass as tile features. The loop is **purely event-driven**: a `crossbeam::select!` parks on the task channel and on a wake channel pinged by the decoder thread on each tile arrival — no timeout-based polling.

3. **Tile fetch + decode pipeline** (`ttymap-engine/src/map/tile/`): Three-layer flow `FetchLane → decoder → TileCache`. The fetch lane runs a fixed worker pool over a priority queue (`fetch/lane.rs`) and delegates per-tile bytes acquisition to a `TileFetcher` impl. Disk cache is a decorator (`fetch/disk_cached.rs`) that wraps the slow inner (today `HttpFetcher`) with read-through / write-through. A dedicated decoder thread (`decoder.rs`) parses MVT bytes off the render thread and forwards `DecodedTile`s to the cache via `mpsc`. The cache also keeps a synchronous **render-thread disk fast path** (`tile::disk` + `cache::DiskFastPath`) that bypasses the worker queue for already-on-disk tiles, which is the hot case during fast pan / zoom.

### Rendering pipeline

`Viewport` → `RenderPipeline::render()` → visible tiles → spatial query (R-tree) → draw features by layer order (fills/lines first, then symbols sorted by priority) → `Canvas` → `MapFrame` (grid of `MapCell { ch, fg, bg }`) → main thread paints via ratatui.

Key modules:
- **`ttymap-engine/src/map/render/renderer.rs`**: Orchestrates tile fetching, spatial queries, and drawing. Determines visible tiles from center/zoom, queries each tile layer's R-tree for on-screen features, draws non-symbol features first then symbols sorted by `sort` key.
- **`ttymap-engine/src/map/tile/decode/`**: Decodes protobuf MVT tiles into `DecodedTile` with per-layer R-trees (`rstar`) for spatial indexing. `mod.rs` owns the public types and the top-level `decode()` entry; `geometry.rs` is the zigzag + command-stream decoder; `tags.rs` decodes per-feature tag pairs; `decompress.rs` sniffs and unwraps gzip.
- **`ttymap-engine/src/map/render/canvas.rs` / `braille.rs`**: 2×4 pixel Braille rendering. Each terminal cell maps to 8 sub-pixels. Supports polyline (with line width via Bresenham), polygon fill (via `earcut` triangulation), and text overlay. Colors use the xterm-256 palette.
- **`ttymap-engine/src/map/render/frame.rs`**: `MapFrame` — the completed grid of `MapCell { ch, fg, bg }` plus the view (center/zoom) it was rendered at, so overlays can project coordinates against the same frame regardless of staleness.
- **`ttymap-engine/src/map/styler/`**: Defines map styles as Rust data structures. `schema/mapscii.rs` is the single rule source (filter expressions, style_type, min/max zoom); themes vary only by `ColorPalette` swap. Applied during tile decode to produce styled `Feature` objects. Future schemas (Protomaps etc.) land as `schema/<name>.rs`.
- **`ttymap-engine/src/theme/`**: Engine-side color data — `palette.rs` (`ColorPalette` + `DARK` / `BRIGHT` consts, xterm-256 indices) and `mod.rs` (`ThemeId`). No ratatui dependency. The renderer's styler reads `ColorPalette`; the binary's UI adapter consumes the same data through `crate::theme::*` re-exports.
- **`ttymap-tui/src/theme/`**: Binary-side ratatui adapter — `ui.rs` (`UiTheme`) and `style.rs` (`StyleKind` semantic tags). `mod.rs` re-exports `ColorPalette` / `ThemeId` / `DARK` / `BRIGHT` from `ttymap_engine::theme` so the rest of the binary keeps using `crate::theme::*` without caring that the data half lives in the engine crate.
- **`ttymap-engine/src/map/tile/cache.rs`**: Orchestrator — LRU memory cache (`lru` crate), view state (center / zoom), prefetch ring, and the channel drain (`poll_completed`). On a memory miss it consults the optional `DiskFastPath` (synchronous disk read + push to decoder, bypassing the worker queue) before enqueueing for the slow lane.
- **`ttymap-engine/src/map/tile/disk.rs`**: Free-function disk read/write helpers used by both `fetch::DiskCachedFetcher` (worker-side, read+write through) and `cache::TileCache` (render-thread fast path, read-only). Layout: `{cache_dir}/{z}/{x}-{y}.pbf`.
- **`ttymap-engine/src/map/tile/fetch/`**: `TileFetcher` (per-backend trait — "key → bytes"), the generic `FetchLane<F>` (queue / workers / dedup / priority), and the `DiskCachedFetcher<F>` decorator that adds a disk read-through / write-through layer to any inner fetcher. `http.rs` is the only inner backend today; new backends (mbtiles, pmtiles, …) add a `TileFetcher` impl + a branch in `app::build_tile_cache`.
- **`ttymap-engine/src/map/tile/decoder.rs`**: Single-thread relay that reads bytes from the fetch lane, calls `decode::decode`, and forwards `DecodedTile`s to the cache. Empty bytes (negative cache from failed fetches) bypass `decode()` and surface as `DecodedTile::empty()`.
- **`ttymap-tui/src/app/dispatcher.rs`**: GoF Receiver for `UserCommand`. Owns the state that mutates in response to commands (map handle, lua handle, compositor, theme, sidebar, cursor, overlay sink) and every handler. ratatui-free — only `App::render_into` touches ratatui.
- **`ttymap-tui/src/app/`**: Loop driver. `App::run` drains the [`AppEvent`] bus, forwards commands to `app::Dispatcher`, and asks ratatui to paint each iteration. Owns the latest `MapFrame` (the rendered product, delivered into the bus by the `FrameSink` callback the binary handed the engine at startup), `MouseAdapter`, and `poll_timeout`. `ttymap-tui/src/app/event.rs` holds `AppEvent` (`Command` / `FrameReady` / `Input` / `Wake`); `ttymap-tui/src/app/ui.rs` is the ratatui draw entry; `ttymap-tui/src/app/frame_timer.rs` is the per-iteration wake source; `ttymap-tui/src/app/frame_widget.rs` is the binary-side `Widget` newtype that adapts engine `MapFrame`s to ratatui's draw protocol (orphan rules force the wrapper). See [docs/design.md](docs/design.md) for the UserCommand-vs-direct-API judgment rules.
- **`ttymap-tui/src/compositor/`**: Stack-based focus/modal system (helix-inspired). One primitive: a stack of `Component`s, where the top owns key focus. Components render side panels through `Component::render` and can emit ops via `Window` (`close` / `open` / `emit` / `ignore`); the compositor drains the queue after each hook and applies them via the single `Op` enum (`Push` / `Close` / `Command` — see `compositor/op.rs`). World-space overlays (markers etc.) are *not* a Component concern — every Lua plugin's per-frame map paint runs through `lua::tick::dispatch_tick` (called from `ui::draw`) which hands the plugin a `MapApi` it draws into directly. Render orchestration lives in `compositor/render.rs` (a free function `paint(...)`); the focus stack itself is ratatui-free. `Placement` has two variants: `Floating` (palette-only, drawn over the map) and `Sidebar` (left rail, equal-vertical-split among up to 3 visible cards). Lua plugins always land in `Sidebar`. **No framework-side dedup**: re-pressing an activation key stacks a fresh instance — toggle behavior is plugin-side policy.
- **`ttymap-tui/src/palette/`**: `:`-triggered command palette as an ephemeral `Component` (state per-open, discarded on pop). Provider sub-modes (theme picker, forward-geocode search) swap in place via `PaletteAction::SwitchProvider` or are pushed pre-loaded by their key activation. Providers can be sync (`OnEachKey` filter — command, theme) or async (`Debounced` filter, `poll()` to drain results, `is_loading()` for the spinner — search).
- **`ttymap-tui/src/cli/`**: CLI subcommands (currently `snap`). Each subcommand is one file with a `run()` entry point. Named `cli/` (not `commands/`) so the GoF Command pattern's `Command` role — represented in this codebase by `UserCommand` (top-level `ttymap-tui/src/command.rs`) — doesn't share a name with the CLI subcommand bucket.
- **`ttymap-tui/src/input/`**: Input subsystem. `thread.rs` is the producer that blocks on `crossterm::event::read()` and pushes `AppEvent::Input` onto the App bus; `keymap.rs` holds the `KeyMap` table + `KeybindingOverrides` (read from `[keymap]` config and Lua `keymap.set`); `mouse.rs` holds the `MouseAdapter` that translates raw mouse events to `UserCommand`. Multi-key sequences are owned by `BaseLayer` (today: `gg` for world view); the keymap itself is stateless. `:` opens the command palette, `/` opens the palette pre-loaded with the search provider. Lives in the binary because crossterm input is a binary-side concern; the engine itself stays IO-free.
- **`ttymap-engine/src/geo.rs`**: Foundation: Web Mercator projection math — lon/lat ↔ tile coordinates, distance calculations.
- **`ttymap-engine/src/map/render/label.rs`**: Collision-free label placement buffer.
- **`ttymap-tui/src/lua/`**: Lua scripted plugins (mlua + Lua 5.4 vendored). All in-tree plugins live here. **"Plugin" is purely a Lua-side concept** — a `.lua` file's worth of `register_palette_command` / `register_keybind` / `on_event` calls. The Rust host has no notion of plugin identity, no per-script slot, no attribution; it just exposes the API surface and lets each `register_*` call push directly into a live registry. nvim-style: a `.lua` file (or `<name>/init.lua`) under `<runtime>/plugin/` is a `require`-able module. Activation is explicit: `runtime/init.lua` requires every bundled plugin; `~/.config/ttymap/init.lua` may add or override. **No disk walker** — init.lua is the only entry point. Plugins join host loops by calling `ttymap.api.frame.on_tick(fn)` (sugar for `ttymap.on_event("tick", fn)`), `ttymap.on_event(name, fn)` for any host event (`frame_ready` / `map_jumped` / `theme_changed` / `resized` / …), `register_palette_command`, or `register_keybind`. **Single shared Lua VM** for the whole subsystem — `build_subsystem(defaults)` is the merged bootstrap: install `ttymap.opt`/`ttymap.keymap`, install API surface (`register_*`, `on_event`, `http`, `map`, `api`, `notify`, plus the host primitive `runtime_path` (resolved layer list)), then run the bundled `runtime/init.lua` only — that file `require("ttymap.plugin_searcher").install()`s the Lua-side plugin searcher up front (resolves `<layer>/plugin/<name>.lua` etc. and runs the chunk as a plain `load(source)()`), then `system opts → bundled plugin requires → require("ttymap.user_config").load()` for user init.lua last. **Rust knows neither the `plugin/` directory layout nor the user-config path** — both live entirely in Lua libs (`runtime/lua/ttymap/plugin_searcher.lua`, `runtime/lua/ttymap/user_config.lua`). `init.lua` can `require "ttymap.<name>"` and mutate a config holder lib at `runtime/lua/ttymap/<name>.lua`; the plugin reads the same cached table when its require fires (Neovim-style). Layout: `ttymap-tui/src/lua/api/` extends the `ttymap` global with the runtime API (Rust→Lua API binding — file ↔ Lua-namespace 1:1: `http.rs` / `json.rs` / `sgp4.rs` / `map.rs` (`HostMap` userdata + `make_map_table` per-frame `on_tick` arg) / `config.rs` / `help.rs` / `log.rs` / `tile.rs`, plus `register.rs` for `register_*` / `on_event` (each call pushes directly into the live registry / bus and returns a Lua-facing handle) and `imperative.rs` for the `ttymap.api.*` runtime cluster); `ttymap-tui/src/lua/bridge/` adapts Lua specs to Rust traits (`Component`, `PaletteProvider`); top-level `mod.rs` (`LuaSubsystem` + `build_subsystem(Config) -> (LuaSubsystem, Config, KeybindingOverrides, KeyMap)`) / `vm.rs` (`new_lua` + `install_builtin_searcher` for libs — `<layer>/plugin/...` resolution lives in the Lua-side `ttymap.plugin_searcher` lib, not here) / `registrar.rs` (`PluginRegistry` — live registry behind `Rc<RefCell<...>>`) / `runtimepath.rs` / `init_lua.rs` (Rust runs only the bundled `<layer>/init.lua` via `run_system_init_lua`; that file pulls in user config itself via `require("ttymap.user_config").load()` — a Lua lib at `runtime/lua/ttymap/user_config.lua` that resolves the path via `XDG_CONFIG_HOME` / `HOME` and `dofile`s it. Rust never names `~/.config/ttymap/init.lua`. `read_init_lua_config_only` is the snap-only thin path; it reuses the same `ttymap.user_config` lib) / `handle.rs` / `map_api.rs` (host-side `MapApi` struct — per-frame draw surface) / `host.rs` (`LuaHostShared` + `LuaHostHandles` + `NotifyEntry` + `HelpEntry` — host-side Lua-runtime state, deliberately outside `api/` so that directory stays namespace-pure) own subsystem orchestration, VM setup, the event bus, runtime layer resolution, the config-DSL state, draw surface, host-side state, and channel plumbing. Handle-returning surfaces (`ttymap.api.card.open` / `ttymap.api.palette.open` / `ttymap.on_event` / `ttymap.api.frame.on_tick` / `ttymap.register_palette_command` / `ttymap.register_keybind`) all return a Lua-facing handle whose `:remove()` (`:close()` for the compositor-stack ones) drops the registration. The registry behind `register_palette_command` / `register_keybind` lives at `Rc<RefCell<PluginRegistry>>` (`registrar.rs`): `BaseLayer` borrows it on each keypress for activation dispatch, the `:` palette-installer borrows it on each open to snapshot a fresh `CommandSeed`, and Lua handles mutably borrow it from `:remove()` to drop entries by ID. See **[docs/lua-architecture.md](docs/lua-architecture.md)** for the full surface — every namespace, activation surfaces, runtime path resolution, config chain, bundled plugins.
- **`ttymap-tui/src/shared/geoip.rs`**: Binary-only — IP→lon/lat lookup used by `--here` and the `here` plugin. The engine doesn't know about geoip; the binary resolves IP to a coordinate up front and hands a plain lat/lon to `engine::map::build`.
- **`ttymap-engine/src/shared/http/`**: User-Agent-tagged `reqwest` wrapper. The engine's tile fetcher (`map/tile/fetch/http.rs`) consumes it directly; the binary's Lua `ttymap.http` bridge (`lua/api/http.rs`) and `geoip.rs` re-borrow it from `ttymap_engine::shared::http` so there's a single source of truth.

## Rust Edition

Uses Rust **2024 edition** — supports `let chains` in `if let` and `while let` natively (no feature flag needed).
