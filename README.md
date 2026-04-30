# ttymap

Terminal-based map viewer. Renders [Mapbox Vector Tiles](https://github.com/mapbox/vector-tile-spec) as Unicode Braille characters with ANSI 256-color in your terminal.

Inspired by [mapscii](https://github.com/rastapasta/mapscii).

## Features

- **Braille rendering** — 2x4 pixel grid per terminal cell for high-resolution maps
- **Vim-style navigation** — `hjkl` pan, `a`/`z` zoom, `gg` world view
- **Mouse support** — drag to pan, scroll to zoom towards cursor
- **Command palette** — `:` opens a fuzzy-filterable popup listing every action
- **Location search** — `/` to search with autocomplete (Nominatim)
- **Jump to current location** — `--here` at startup (sync) or from the palette (async) via IP geolocation
- **Wikipedia panel** — `i` to show nearby Wikipedia articles, Enter to jump
- **Cursor readout** — live lat/lon under the mouse cursor
- **Place name display** — reverse geocoding shows current location
- **Scale bar + attribution** — always on screen
- **Help popup** — `?` shows all keybindings
- **Frame export** — palette entry writes the current view as an ANSI-coloured text file under `~/.local/share/ttymap/exports/`
- **Configurable** — keybindings, initial position, language via Lua `init.lua` (Neovim-style)
- **Lua plugin API** — every in-tree plugin is a Lua script under `runtime/lua/`; drop a `*.lua` file into `~/.config/ttymap/plugins/` to add one without rebuilding, or `~/.config/ttymap/lua/<name>.lua` to *shadow* a bundled plugin by stem. Bundled and user scripts share one dispatcher and the same `host:*` accessor surface.

## Usage

**Interactive:**

```bash
cargo run                                         # default position
cargo run -- --lat 35.68 --lon 139.76 --zoom 10   # Tokyo
cargo run -- --here                               # jump to IP-based current location on startup
cargo run -- --style bright                       # bright theme
```

**Headless snapshot** (`snap` / `snapshot`) — render a single frame as ANSI text and write it to stdout or a file. Pipe-friendly for dashboards, cron jobs, README illustrations, email attachments:

```bash
# Tokyo to stdout (defaults to the current terminal size)
ttymap snap --lat 35.68 --lon 139.76 --zoom 12

# Write to a file
ttymap snap --lat 35.68 --lon 139.76 --zoom 12 -o tokyo.ans
cat tokyo.ans                                    # replay later

# IP-geolocated center, explicit size
ttymap snap --here --cols 120 --rows 40

# Bright theme, bigger canvas, longer fetch timeout for slow networks
ttymap snap --lat 48.86 --lon 2.35 --zoom 14 \
    --style bright --cols 160 --rows 50 --timeout-ms 15000

# Alias
ttymap snapshot --lat 40.71 --lon -74.01 --zoom 12 > nyc.ans
```

`snap` emits raw xterm-256 ANSI escape codes; `cat` the file into any compatible terminal, or pipe directly (`ttymap snap … | less -R`).

### Keybindings

Press `?` for the in-app cheatsheet — it reflects the live keymap and any plugin keys that are loaded. Keybindings can be overridden in `~/.config/ttymap/init.lua` via `ttymap.keymap.set(action, keys)`.

## Architecture

```
src/
├── main.rs              CLI entry + interactive-mode composition
├── lib.rs               crate root
├── logging.rs           XDG state log
├── config.rs            Config struct + Default impls (loaded from init.lua)
├── keymap.rs            KeyBinding → AppMsg table + user overrides
├── geo.rs               Web Mercator, projection, distance
│
├── theme/                colour palette + ratatui adapter
│   ├── mod.rs            ThemeId + re-exports
│   ├── palette.rs        ColorPalette struct + DARK / BRIGHT consts (xterm-256)
│   └── ui.rs             UiTheme (ratatui style adapter)
│
├── app/                 App struct + event loop + message dispatch
│   ├── mod.rs           App::new / run / dispatch — single side-effect boundary
│   ├── msg.rs           AppMsg enum (Map / Jump / SetTheme / CycleFocus / …)
│   └── mouse.rs         MouseAdapter: MouseEvent → Vec<AppMsg>
│
├── commands/            one file per CLI subcommand (main.rs stays thin)
│   ├── mod.rs           Command enum + run() dispatch
│   └── snap.rs          `ttymap snap` / `snapshot` — headless single-frame renderer
│
├── compositor/          helix-inspired focus / modal stack
│   ├── mod.rs           Component, Compositor, Registrar, Activation, PaletteEntry, Task, Context
│   ├── base.rs          BaseLayer — keymap + activation dispatch + gg sequence
│   ├── layout.rs        PanelAnchor (anchor vocabulary for module.layout)
│   ├── map_api.rs       MapApi — world-space + screen-space draw primitives for paint_on_map
│   └── window.rs        Window (event-side, queues ops) + RenderWindow (render-side, owns UiTheme)
│
├── palette/             `:`-triggered universal picker (itself a Component)
│   ├── mod.rs           CommandPalette + install(&mut Registrar)
│   ├── panel.rs         popup layout
│   └── provider/        default provider + theme sub-mode
│
├── lua/                 Lua bridge (mlua + Lua 5.4 vendored).
│   ├── mod.rs           BUILTIN_SCRIPTS + BUILTIN_LIB_SCRIPTS arrays, register_one dispatcher,
│   │                    custom package.searchers entry resolving `require "ttymap.*"` against
│   │                    binary-embedded lib scripts (Neovim-style, no runtime install needed)
│   ├── component.rs     LuaComponent — Component impl backed by a Lua module
│   ├── palette_provider.rs  LuaPaletteProvider — PaletteProvider impl (search)
│   ├── host.rs          LuaHostShared + host:* accessors (fetch_url / jump / close /
│   │                    export_frame / center / parse_json / url_encode / attribution /
│   │                    geoip_endpoint / keymap_entries / plugin_palette_entries)
│   └── map_api.rs       Lua-side MapApi bridge (point / label / text_anchored / center / zoom / cursor / area_width)
│

├── map/                 domain — viewport state + rendering pipeline
│   ├── state.rs, action.rs, mod.rs
│   ├── render/          tiles → MapFrame on a dedicated thread
│   │   ├── pipeline.rs, thread.rs, renderer.rs
│   │   ├── canvas.rs, braille.rs, frame.rs, frame_widget.rs
│   │   └── view.rs, label.rs, geom/, earcut_worker.rs, panic_silence.rs
│   ├── styler/          Mapbox GL-style rules — schema/mapscii.rs single source; theme swaps ColorPalette only
│   └── tile/            MVT fetch + cache + decode
│       ├── cache.rs         Memory LRU + view state + prefetch (orchestrator) + disk fast path
│       ├── decoder.rs       Relay thread: bytes → DecodedTile (off the render thread)
│       ├── disk.rs          On-disk tile read/write helpers (shared by fast path + decorator)
│       ├── decode/          Protobuf → DecodedTile (geometry / tags / decompress sub-modules)
│       └── fetch/           TileFetcher trait + FetchLane (queue/workers/dedup) + http + disk decorator
│
├── shared/              host-and-Lua-bridge utilities
│   ├── geoip.rs         IP-based lat/lon lookup (also used by snap CLI)
│   └── http/            user-agent-tagged reqwest wrapper
│
└── ui.rs                non-modal UI shell — draw() forwards to the Compositor

runtime/
└── lua/                 bundled Lua plugin scripts (aircraft, attribution, export,
    │                    help, here, info, quake, satellite, scalebar, search, wiki).
    │                    `include_str!`'d at compile time by `BUILTIN_SCRIPTS`; the
    │                    binary ships them as data, not Rust source.
    └── ttymap/          shared lib scripts (fmt, …). Resolved via `require "ttymap.X"`
                         through a custom `package.searchers` entry — bundled into the
                         binary alongside the plugins, no runtime install needed.
```

### Layering

- **`map/`** — domain. Knows nothing about UI, plugins, or focus. `Action` carries every map-level mutation, including mouse-continuous variants (`PanCells`, `ZoomAt`).
- **`app/`** — the **controller**. `AppMsg` (in `app/msg.rs`) is the closed enum every input source (keymap, palette, compositor components, mouse adapter, async tasks) emits; `App::dispatch` in `app/mod.rs` is the sole place that executes them. Command pattern with `App` as the Receiver — see [`docs/design.md`](docs/design.md) for the AppMsg-vs-direct-call judgment rules.
- **`compositor/`** — focus and modal state. A stack of `Component`s; the top is focused. No `is_visible` / `activate` / `deactivate` contract — presence on the stack *is* the lifecycle. `Tab` / `Shift-Tab` cycle focus (framework-reserved, intercepted before any component sees them).
- **`app/mouse.rs`** — pure adapter. `MouseEvent → Vec<AppMsg>` (`CursorMoved` on every event; drag → `Map(PanCells)`; scroll → `Map(ZoomAt)`). No state mutation. Lives under `app/` because it's part of the dispatch pipeline, not a UI concern.
- **`ui.rs`** — non-modal shell. `draw()` paints the latest `MapFrame`, lets every Component on the stack stamp its `paint_on_map` markers, then forwards modal rendering to the Compositor. Always-on overlays (info, attribution, scale bar) are themselves Components registered via `Registrar::add_overlay` — they paint after the regular stack but never receive key events.
- **`palette/`** — `:`-triggered universal picker. Itself a `Component`; its provider table is harvested from the `Registrar` at boot so plugins' palette entries appear automatically. Palette installs last so it sees everyone else's entries.
- **`lua/`** — every in-tree plugin. `BUILTIN_SCRIPTS` lists `(stem, include_str!(...))` pairs; one dispatcher (`register_one`) reads each script's module metadata (`kind` / `activation` / `key` / `label` / `enabled`) and wires it. User plugins under `~/.config/ttymap/plugins/` flow through the same dispatcher. Runtime data (attribution, geoip endpoint, live keymap, palette hints) is exposed via `host:*` accessors backed by `Arc<LuaHostShared>`. The compositor never names a concrete plugin type; Rust never knows a specific plugin's name. Drawing primitives (`MapApi`) and layout vocabulary (`PanelAnchor`) live under `compositor/` — the Lua bridge thin-wraps them on each `paint_on_map` call.
- **`theme/`** — palette data + `UiTheme` ratatui adapter + `StyleKind` semantic tags. Lua scripts ask for a tag string ("accent" / "muted" / …) and the bridge resolves it through the active `UiTheme` to a concrete `ratatui::Style`. Lua plugins never see `UiTheme` directly.

### Message flow

```
raw event
  ↓ keyboard / mouse / async poll / tile arrival
  ↓ produces 0..N AppMsg (pure translation)
  ↓
App::dispatch(msg)
  ↓
    AppMsg::Map(action)      → MapState::process_action(&action)
    AppMsg::Jump(loc)        → MapState::jump_to(loc)
    AppMsg::SetTheme(id)     → App::apply_theme (rebuilds styler + UI theme)
    AppMsg::CursorMoved(c,r) → overlay.set_cursor
    AppMsg::CycleFocus(fwd)  → Compositor::cycle
    AppMsg::Resize(cols,rows)→ App::handle_resize
```

Keyboard and mouse take different paths to `AppMsg` — keys go through the Compositor; mouse events go through a pure adapter:

```
key event
  ↓ Compositor::handle_event(event, ctx):
    [reserved]  Tab / Shift-Tab   → AppMsg::CycleFocus(…)
    [focused]   focused component's handle_event(event, &mut win)
                  ↓ win.emit / win.open / win.close / win.ignore
    [fallback]  only if the focused component called win.ignore()
                and focus isn't already on BaseLayer
                → re-deliver to BaseLayer (keymap + activation table)
  ↓ Vec<AppMsg>

mouse event
  ↓ MouseAdapter::translate(event) → Vec<AppMsg>:
    every event   → AppMsg::CursorMoved(col, row)
    drag (left)   → AppMsg::Map(Action::PanCells(dx, dy))
    scroll        → AppMsg::Map(Action::ZoomAt { anchor_*, zoom_in })
```

### Render flow

Rendering is decoupled from fetching. The render thread builds a `MapFrame` from the current `Viewport`; the main thread consumes it. Stale frames are fine — overlays reproject against the frame's own center/zoom.

```
main thread (ratatui draw):
  ui::draw(f, &compositor, &theme, &ctx):
    1. latest MapFrame is rendered into the map area
    2. MapApi set up; compositor.paint_on_map(map_api)
       — every Component on the stack paints world-space primitives
         (wiki / aircraft / satellite / quake markers, info chrome, scale bar, …)
    3. compositor.render(f, area, theme, ctx)
       — every Component on the stack drawn bottom-up; focused last
         so its panel sits on top
    4. always-on overlays painted after the stack as a final pass
    5. footer hints from the focused component
```

### Focus model

Focus is a `focused_idx` into the Compositor stack, **decoupled from stack position**. Pushing a modal puts focus on it; `Tab` moves focus back to the base layer without popping the modal (the old `Focus::Background` behaviour). Stack order never changes through cycling — only which component receives keys first.

Dedup is by `Any::type_id` for Rust components — pressing an activation key while the plugin is already on the stack focuses the existing instance instead of stacking a duplicate; the concrete type *is* the identity, no per-plugin declaration needed. Adapters that share a single Rust type across many plugin instances (notably `LuaComponent`, where every Lua-driven plugin is wrapped in the same struct) override `Component::dedup_tag` to return a per-instance string, so different Lua plugins coexist on the stack while re-toggling the same one still closes it.

### Plugin API

A plugin is a Lua module — a table the script returns. Module metadata drives wiring; the dispatcher never special-cases a name.

```lua
return {
    name = "wiki",                              -- required identifier
    activation = "toggle",                      -- "toggle" (default) | "overlay" | "spawn"
    key = "i",                                  -- optional activation key char
    label = "Toggle wiki",                      -- palette entry label
    enabled = true,                             -- default true; opt-out hook
    layout = { anchor = "right", width = 56 },  -- panel placement (Component only)
    footer_hints = { { "C-n/C-p", "select" } }, -- shown while focused

    render       = function() ... end,                -- panel: list of strings or styled-span tables
    paint_on_map = function(map) ... end,             -- world-space primitives (point / label / text_anchored)
    handle_event = function(key) ... end,             -- return nil / { close = true } / { ignore = true }
    poll         = function() ... end,                -- tick-driven async (host:fetch_url etc.)
}
```

Per-frame `host` and `map` accessors (a partial list):

| | |
|---|---|
| `host:fetch_url(url) -> Job` | background HTTP GET; poll `job:try_take()` |
| `host:parse_json(s)` | JSON → nested Lua tables |
| `host:jump(lon, lat)` | recentre the map |
| `host:close()` | pop self off the compositor stack |
| `host:export_frame()` | dump current frame as ANSI |
| `host:center()` | live map centre |
| `host:attribution()` / `host:geoip_endpoint()` | runtime config |
| `host:keymap_entries()` / `host:plugin_palette_entries()` | what help reads to build the cheatsheet |
| `map:point(lon, lat, glyph, color)` | draw a marker |
| `map:label(lon, lat, text, color)` | draw text next to a point |
| `map:text_anchored(anchor, row, text, color)` | corner-anchored text (info / scalebar / attribution) |
| `map:center()` / `map:zoom()` / `map:cursor()` / `map:area_width()` | frame state |

To register as a palette provider (search uses this), expose a `palette = { prompt, submit_mode, filter, items, execute, poll, is_loading }` sub-table on the returned module. The dispatcher reads palette-provider semantics from the *shape* of the returned table — there is no separate `kind` field.

For a related family of toggles in one file, return `{ entries = { module_a, module_b, … } }` instead of a single module. Each entry is itself a plugin module (same shape as above) and registers as its own palette entry, with per-entry `name` driving the compositor's stack-dedup so distinct entries coexist. File-level `enabled = false` skips the whole pack; per-entry `enabled = false` skips just that entry. The bundled `satellite.lua` uses this to ship ISS + Hubble from one file via the shared `ttymap.satellites` factory.

Adding a bundled plugin = drop a `.lua` under `runtime/lua/` + 1 line in `BUILTIN_SCRIPTS`. Adding a user plugin = drop a `.lua` into `~/.config/ttymap/plugins/`; the file *is* the config, so `enabled = false` in the returned table is how you turn it off without removing the file. Errors in any callback are logged, not propagated — a buggy plugin can't take the host down.

### Concurrency

| Thread | Responsibility |
|--------|----------------|
| main | event loop, compositor, Lua dispatch, UI state, terminal draw |
| render | MapFrame generation (tile fetch + draw) |
| tile fetch | HTTP workers with priority queue |
| Lua `host:fetch_url` | one short-lived OS thread per request (Nominatim / Wikipedia / geoip / ADS-B / TLE / USGS) — Lua side polls `job:try_take()` |

mpsc channels connect the threads; the main thread never blocks on I/O.

## Roadmap

ttymap aims to be a **modern Rust replacement for mapscii** — still a terminal map viewer at heart, but with a first-class plugin story so the interesting overlays (planes, ships, weather, …) live outside the core.

### Principles

- **Core stays lean.** A map viewer, not a GIS platform. The core handles tiles, projection, rendering, navigation. Anything domain-specific is a Lua plugin.
- **Plugin-first.** Every built-in (info / scalebar / attribution / aircraft / satellite / quake / wiki / here / search / export / help) is a Lua script — the bridge dogfoods itself.
- **Boring where it matters.** Stable protocols (MVT, OSM, TOML), predictable resource use, `cargo install` ships a single binary.

### Short-term

- **Tile backends** ([#30](https://github.com/Kohei-Wada/ttymap/issues/30) MBTiles, [#31](https://github.com/Kohei-Wada/ttymap/issues/31) PMTiles) — offline and CDN-friendly serving. Today the only backend is `mapscii.me`.
- **Error handling policy** ([#17](https://github.com/Kohei-Wada/ttymap/issues/17)) — normalize how soft errors (network, parse) surface.

### Plugin candidates

Already bundled (each is one `.lua` file): live aircraft overlay (OpenSky), TLE-driven satellite tracker (`satellite.lua` shows ISS + Hubble in one panel with per-sat key toggles; append any NORAD ID), USGS earthquakes, Wikipedia geosearch, Nominatim search, IP-geolocate, frame export. The following are open ideas — each can ship as a script under `~/.config/ttymap/plugins/` without touching the core:

- **Live vessel overlay** — AIS via `rtl-ais` / `aisstream.io` ([#26](https://github.com/Kohei-Wada/ttymap/issues/26))
- **Weather** — radar, temperature, wind
- **GeoJSON overlay** ([#33](https://github.com/Kohei-Wada/ttymap/issues/33)) — drop a GeoJSON file in, see it drawn
- **Demo / tour mode** ([#34](https://github.com/Kohei-Wada/ttymap/issues/34)), **hover tooltip** ([#35](https://github.com/Kohei-Wada/ttymap/issues/35)), **multi-line labels** ([#36](https://github.com/Kohei-Wada/ttymap/issues/36))
- **Layer toggle** ([#41](https://github.com/Kohei-Wada/ttymap/issues/41)) — toggle borders / labels / roads / …
- **Terrain / hillshade** ([#45](https://github.com/Kohei-Wada/ttymap/issues/45))
- **Markers from stdin / file** ([#39](https://github.com/Kohei-Wada/ttymap/issues/39))

### Contributing

ttymap is small, the code is documented, and the roadmap is deliberately open. If you want to:

- **Add a feature to core** — open an issue first to sanity-check it isn't plugin material.
- **Write a plugin** — every in-tree plugin is a Lua script under `runtime/lua/`. Drop a `*.lua` file into `~/.config/ttymap/plugins/` to add one without rebuilding; the file *is* the config (set `enabled = false` on the returned table to disable). The simplest fetch+render example is `quake.lua`; for a full panel + selection + modal detail flow see `wiki.lua`; for a debounced palette picker see `search.lua`. The bridge surface is documented in the `src/lua/` module-level docs.
- **Fix a bug or clean something up** — PRs welcome. The pre-commit hook runs tests, clippy, and rustfmt; follow its lead.

Issues on GitHub carry the current opinion of what's easy, what's hard, and what's deferred. Skim them before designing.

## Configuration

Config file: `~/.config/ttymap/init.lua` (Neovim-style)

```lua
ttymap.opt.map.lat            = 35.6828
ttymap.opt.map.lon            = 139.7595
ttymap.opt.map.zoom           = 10
ttymap.opt.render.language    = "ja"

-- IP-based geolocation (shared by `--here` flag and the `here` plugin)
ttymap.opt.geoip.on_startup   = false
ttymap.opt.geoip.endpoint     = "https://ipapi.co/json/"
ttymap.opt.geoip.timeout_ms   = 2000

ttymap.keymap.set("zoom_in", { "i", "+" })
ttymap.keymap.set("quit",    { "q", "C-q" })

-- Conditional / computed config (the killer feature over TOML):
local heavy = os.getenv("TTYMAP_HEAVY") ~= nil
ttymap.opt.cache.memory_tiles = heavy and 2048 or 512
```

Every option is optional; omitted values stay at their built-in defaults.
Errors in `init.lua` (syntax, type mismatch, runtime exception) are
logged and recovered — the app keeps booting with defaults. Per-plugin
behaviour still lives inside each `.lua` script — drop a copy under
`~/.config/ttymap/lua/<name>.lua` to shadow a bundled plugin by stem.

### Migrating from `config.toml`

| Old TOML | New init.lua |
|---|---|
| `[map] lat = 35.68` | `ttymap.opt.map.lat = 35.68` |
| `[render] style = "bright"` | `ttymap.opt.render.style = "bright"` |
| `[cache] memory_tiles = 1024` | `ttymap.opt.cache.memory_tiles = 1024` |
| `[geoip] on_startup = true` | `ttymap.opt.geoip.on_startup = true` |
| `[keymap] zoom_in = ["i", "+"]` | `ttymap.keymap.set("zoom_in", { "i", "+" })` |

## Install

ttymap ships a `runtime/` directory of bundled Lua plugins; the
binary loads them from disk at startup, in the same shape as helix
or nvim. The Makefile installs both the binary (via `cargo install`)
and the runtime:

```bash
git clone https://github.com/Kohei-Wada/ttymap
cd ttymap
make install
```

This installs:

- `~/.cargo/bin/ttymap` (binary, via `cargo install --path .`)
- `~/.local/share/ttymap/lua/` (bundled Lua plugins + libs)

Single-user, no root. System-wide layouts (`/usr/local/share/ttymap`,
`/etc/ttymap`) are intentionally unsupported — ttymap is a per-user
TUI and root-installs aren't worth the path-juggling.

`cargo install` **alone** is not enough — the binary will fail fast
with a "did you `make install`?" message because the runtime hasn't
been placed. Run `make install-runtime` (or `make install`) to fix.

### Runtime path resolution

The binary builds an ordered list of runtime layers (Neovim-style
runtimepath). Higher layers shadow lower ones — drop a
`~/.config/ttymap/lua/wiki.lua` to replace bundled `wiki`.

1. `$TTYMAP_RUNTIME` — env override (optional escape hatch)
2. `$CARGO_MANIFEST_DIR/runtime` — `cargo run` from a git checkout (dev wins over stale install)
3. `$XDG_CONFIG_HOME/ttymap` (default `~/.config/ttymap`) — user overrides
4. `$XDG_DATA_HOME/ttymap` (default `~/.local/share/ttymap`) — `make install` target

A layer counts only when it has a `lua/` subdirectory. The user-tier
is empty by default; you opt in by creating `~/.config/ttymap/lua/`.

## Build

```bash
cargo build       # build.rs compiles proto/vector_tile.proto via protox
cargo test
cargo clippy
```

## File locations

| Path | Content |
|------|---------|
| `~/.config/ttymap/init.lua` | Configuration (Neovim-style Lua) |
| `~/.config/ttymap/plugins/` | User Lua plugins (drop `*.lua` here) |
| `~/.config/ttymap/lua/` | User overrides for bundled plugins / libs (shadow by stem) |
| `~/.local/share/ttymap/lua/` | Bundled Lua plugins + libs (placed by `make install`) |
| `~/.cache/ttymap/` | Disk tile cache |
| `~/.local/state/ttymap/ttymap.log` | Log file (auto-rotated at 1MB) |

## License

Dual-licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or
  <http://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or
  <https://opensource.org/licenses/MIT>)

at your option.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally
submitted for inclusion in the work by you, as defined in the Apache-2.0
license, shall be dual-licensed as above, without any additional terms
or conditions.
