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
- **Configurable** — keybindings, initial position, language via TOML config
- **Plugin API** — built-in features (search, wiki, here) use the same `Plugin` trait external plugins will

## Usage

```bash
cargo run                                         # default position
cargo run -- --lat 35.68 --lon 139.76 --zoom 10   # Tokyo
cargo run -- --here                               # jump to IP-based current location on startup
cargo run -- --style bright                       # bright theme
ttymap clear-cache                                # clear disk tile cache
```

### Keybindings

| Key | Action |
|-----|--------|
| `h` `j` `k` `l` / Arrow keys | Pan |
| `w` `b` | Fast pan (left/right) |
| `Ctrl-d` `Ctrl-u` | Fast pan (down/up) |
| `a` `+` / `z` `-` | Zoom in / out |
| `gg` | Zoom to world |
| `0` | Reset to initial position |
| `:` | Command palette |
| `/` | Search location (autocomplete) |
| `i` | Toggle Wikipedia panel |
| `?` | Toggle help |
| `Tab` / `Shift-Tab` | Cycle focus across visible plugins |
| `q` / `Ctrl-C` | Quit |

**Mouse:**

| Action | Effect |
|--------|--------|
| Drag | Pan |
| Scroll | Zoom towards cursor |
| Move | Live cursor lat/lon in the info readout |

**Search mode (`/`):**

| Key | Action |
|-----|--------|
| Type | Filter locations |
| `Enter` | Execute search |
| `↑` `↓` / `Ctrl-N` `Ctrl-P` | Navigate results |
| `Ctrl-U` | Clear query |
| `Esc` | Cancel |

**Wikipedia panel (`i`):**

| Key | Action |
|-----|--------|
| `Ctrl-N` `Ctrl-P` | Navigate articles |
| `Enter` | Open article detail / jump to location |
| `r` | Refresh from current map center |
| `Esc` | Close detail / close panel |

**Command palette (`:`):**

| Key | Action |
|-----|--------|
| Type | Filter commands (substring match on label) |
| `↑` `↓` / `Ctrl-N` `Ctrl-P` | Move selection |
| `Enter` | Run the selected command |
| `Ctrl-U` | Clear query |
| `Esc` | Cancel |

Keybindings are customizable via `~/.config/ttymap/config.toml`.

## Architecture

```
src/
├── main.rs           CLI entry + subcommands
├── lib.rs            crate root
├── app.rs            App struct, event loop, composition root
├── config.rs         TOML config + CLI overrides
├── logging.rs        XDG state log
│
├── keyboard.rs       key dispatch (focus-aware routing)
├── mouse.rs          mouse dispatch
├── keymap.rs         key → Action translation + user overrides
├── geo.rs            Web Mercator, MapProjection, distance
├── color_palette.rs xterm-256 color tables (ThemeId + DARK/BRIGHT)
│
├── core/             domain — map state + commands
│   ├── action.rs     Action enum (map-level only)
│   └── state.rs      Core, CoreOptions, RenderRequest
│
├── plugin/           plugin API + built-in plugins
│   ├── mod.rs        Plugin trait, PluginCtx, PluginAction, PluginRegistry
│   ├── help.rs       help popup
│   ├── here/         IP-geolocation "jump to here" (headless, palette-only)
│   ├── search/       forward-geocode popup
│   └── wiki/         nearby-Wikipedia panel
│
├── render/           tiles → MapFrame on a dedicated thread
│   ├── pipeline.rs   RenderPipeline
│   ├── thread.rs     RenderHandle
│   ├── renderer.rs   Feature[] → Canvas
│   ├── canvas.rs     Braille drawing primitives
│   ├── braille.rs    2×4 pixel buffer
│   ├── frame.rs      MapFrame DTO
│   ├── view.rs       Visible-tile math
│   ├── label.rs      R-tree label collision buffer
│   └── geom/         Bresenham, clipping
│
├── tile/             MVT fetch + cache + decode
│   ├── cache.rs      Memory + disk LRU
│   ├── decode.rs     Protobuf → DecodedTile
│   └── fetch/        TileClient trait + mapscii HTTP backend
│
├── styler/           Mapbox GL-style rules (dark / bright presets)
│
├── shared/           cross-cutting utilities
│   ├── async_job.rs  fire-and-poll background job (reused by geocode/wiki/here)
│   ├── geoip.rs      IP-based lat/lon lookup (shared by --here and the here plugin)
│   ├── http/         user-agent-tagged reqwest wrapper
│   ├── nominatim.rs  forward + reverse geocoding
│   └── throttle.rs
│
└── ui/               terminal UI framework
    ├── mod.rs        UiState + draw()
    ├── focus.rs      Focus enum (Map | Plugin(tag) | Palette)
    ├── map_view.rs   MapFrame ratatui adapter
    ├── painter.rs    MapPainter — plugins' world-space drawing API
    ├── theme.rs      UI color set
    │
    ├── palette/      command palette (builtin, not a Plugin)
    │   ├── mod.rs    CommandPalette + PaletteOutcome
    │   ├── commands.rs  static ACTIONS table
    │   ├── panel.rs     ratatui Table popup
    │   └── state.rs     query buffer + substring filter
    │
    └── overlay/      built-in, always-on map decorations
        ├── attribution.rs   © OpenStreetMap
        ├── scale_bar.rs     distance ruler
        └── info/            center/cursor/zoom/place readout
            ├── mod.rs
            └── service.rs   async reverse-geocoder
```

### Layering

- **`core/`** — domain state. Knows nothing about UI or plugins. `Action` only carries map-level commands (Pan, Zoom, Quit, ResetPosition, …); plugin activation is a separate concern.
- **`ui/overlay/`** — identity decorations (info, attribution, scale bar). Always rendered; not plugin territory.
- **`ui/palette/`** — command palette. A **builtin**, not a `Plugin`, because it inherently coordinates across every plugin and would violate the self-contained-widget contract. It enumerates `PluginRegistry` live on activation to build its command list.
- **`plugin/`** — the plugin surface. Built-in plugins (search, help, wiki, here) implement the `Plugin` trait and are registered into the same `PluginRegistry` that will host external plugins. The keyboard handler dispatches by focus + activation-key lookup, never by plugin name.

### Input flow

```
key event
  ↓ keyboard.handle():
    [1a] if palette has focus → route to palette → PaletteOutcome dispatches
    [1b] otherwise focused plugin sees the key first
    [2]  Tab / Shift-Tab cycle focus across visible plugins
    [3]  `:` opens the command palette (builtin activation)
    [4]  registry activation lookup (plugins own their activation keys)
    [5]  keymap.resolve(code, mods) → Action
    [6]  core.process_action(&action)
```

```
mouse event
  ↓ mouse.handle():
    search focused? ignore.
    update core (drag → pan, scroll → zoom)
    notify InfoOverlay of cursor position
```

### Render flow

```
main thread (ratatui draw):
  ui::draw(f, &ui):
    1. map_view renders the latest MapFrame
    2. MapPainter set up; plugins.paint_on_map(painter)
       — wiki plots article markers via painter.point(...)
    3. built-in overlays (info, attribution, scale_bar) stamp their
       rectangles onto the buffer
    4. focused plugin's panel (search popup / help / wiki)
    5. footer hints from the focused plugin (or default)
```

Rendering is decoupled from fetching. The render thread produces a `MapFrame` from the current `RenderRequest`; the main thread consumes it. Stale frames are fine — overlays reproject against the frame's own center/zoom.

### Focus model

`UiState.focus: Focus` is the single source of truth for which plugin (if any) owns the keyboard. Plugins never carry their own `active` flag — rendering, hint selection, and modality all consult `focus`. Activating one plugin implicitly `deactivate`s the previously-focused plugin, so lingering state (wiki markers, etc.) is cleared.

### Plugin API (built-ins + external plugins)

```rust
trait Plugin {
    fn tag(&self) -> &str;
    fn description(&self) -> &str;                 // label shown in palette + help
    fn activation_keys(&self) -> Vec<&'static str>;
    fn activate(&mut self, ctx: &mut PluginCtx);
    fn deactivate(&mut self);
    fn visible(&self) -> bool;                     // is the panel on screen?
    fn handle_key(&mut self, code, mods, ctx) -> PluginAction;
    fn poll(&mut self) -> bool;                    // drain async work; redraw hint
    fn pending_command(&mut self) -> Option<Command>;  // async-emitted command (e.g. Jump)
    fn render(&self, f, area, theme);              // focused / visible panel
    fn footer_hints(&self) -> Vec<(&str, &str)>;
    fn paint_on_map(&self, p: &mut MapPainter);    // world-space primitives
}
```

All methods except `tag` and `handle_key` have defaults, so a passive
data-only plugin (e.g. a map-marker feed) can implement just `tag` +
`paint_on_map` and skip the UI-heavy parts. A plugin with a non-empty
`description()` is automatically listed in the command palette.

`MapPainter` hides projection, buffer, and theme behind primitives like `point(ll, glyph, fg)` — plugins never compute screen coordinates themselves.

### Concurrency

| Thread | Responsibility |
|--------|----------------|
| main | event loop, UI state, terminal draw |
| render | MapFrame generation (tile fetch + draw) |
| tile fetch | HTTP workers with priority queue |
| geocode | Nominatim / Wikipedia calls |

mpsc channels connect the threads; the main thread never blocks on I/O.

## Roadmap

ttymap aims to be a **modern Rust replacement for mapscii** — still a terminal map viewer at heart, but with a first-class plugin story so the interesting overlays (planes, ships, weather, …) live outside the core.

### Principles

- **Core stays lean.** A map viewer, not a GIS platform. The core handles tiles, projection, rendering, navigation, and a small palette of general-purpose built-ins. Anything domain-specific is a plugin.
- **Plugin-first.** Every built-in (search, wiki, here, help) uses the same trait external plugins will. Built-ins dogfood the API.
- **Boring where it matters.** Stable protocols (MVT, OSM, TOML), predictable resource use, `cargo install` ships a single binary.

### Short-term

- **Config sections** ([#47](https://github.com/Kohei-Wada/ttymap/issues/47)) — move flat TOML into `[map]` / `[render]` / `[geoip]` / … before more plugins accumulate config fields.
- **Tile backends** ([#30](https://github.com/Kohei-Wada/ttymap/issues/30) MBTiles, [#31](https://github.com/Kohei-Wada/ttymap/issues/31) PMTiles) — offline and CDN-friendly serving. Today the only backend is `mapscii.me`.
- **Error handling policy** ([#17](https://github.com/Kohei-Wada/ttymap/issues/17)) — normalize how soft errors (network, parse) surface.

### Mid-term — external plugin architecture

The current `Plugin` trait is in-process Rust. To let contributors ship plugins without touching this repo or matching an unstable ABI, the plan is:

1. **Ingest markers from stdin / file** ([#39](https://github.com/Kohei-Wada/ttymap/issues/39)) — the minimum-viable external plugin entry point:
   ```bash
   my-decoder | ttymap --markers -
   ```
   Anything that can produce `{"lat":..,"lon":..,"label":..}` lines becomes a plugin.

2. **Subprocess plugin architecture** ([#32](https://github.com/Kohei-Wada/ttymap/issues/32)) — ttymap spawns plugin processes declared in `config.toml`; line-delimited JSON over stdio for viewport events (ttymap → plugin) and marker / overlay updates (plugin → ttymap). Language-agnostic, sandboxed by the OS process boundary.

3. **Declarative plugin config** — install-by-spec in `config.toml`; no dynamic code loading inside the core repo.

**Rust dylib and WASM plugin paths are explicitly out of scope** until there's a compelling use case. `cdylib` would pin ttymap to a Rust ABI it can't promise; WASM is overkill for line-based data feeds.

### Long-term — plugin candidates (not core features)

The following are fun ideas, but belong **outside this repo** as separate plugin projects once the subprocess architecture lands:

- **Live aircraft overlay** — ADS-B via `dump1090` / `readsb` / OpenSky ([#25](https://github.com/Kohei-Wada/ttymap/issues/25))
- **Live vessel overlay** — AIS via `rtl-ais` / `aisstream.io` ([#26](https://github.com/Kohei-Wada/ttymap/issues/26))
- **Weather** — radar, temperature, wind
- **Seismic / disaster feeds** — USGS earthquake, lightning, tropical storms
- **GeoJSON overlay** ([#33](https://github.com/Kohei-Wada/ttymap/issues/33)) — drop a GeoJSON file in, see it drawn
- **Demo / tour mode** ([#34](https://github.com/Kohei-Wada/ttymap/issues/34)), **hover tooltip** ([#35](https://github.com/Kohei-Wada/ttymap/issues/35)), **multi-line labels** ([#36](https://github.com/Kohei-Wada/ttymap/issues/36))
- **Layer toggle** ([#41](https://github.com/Kohei-Wada/ttymap/issues/41)) — toggle borders / labels / roads / …
- **Terrain / hillshade** ([#45](https://github.com/Kohei-Wada/ttymap/issues/45))

### Contributing

ttymap is small, the code is documented, and the roadmap is deliberately open. If you want to:

- **Add a feature to core** — open an issue first to sanity-check it isn't plugin material.
- **Write a plugin** — the simplest real example is `src/plugin/here/mod.rs` (no UI, one palette command, async background job). A plugin with a non-empty `description()` lands in the command palette automatically. Once the subprocess architecture lands, plugins can live in their own repos.
- **Fix a bug or clean something up** — PRs welcome. The pre-commit hook runs tests, clippy, and rustfmt; follow its lead.

Issues on GitHub carry the current opinion of what's easy, what's hard, and what's deferred. Skim them before designing.

## Configuration

Config file: `~/.config/ttymap/config.toml`

```toml
language = "ja"
lat = 35.6828
lon = 139.7595
zoom = 10.0
wiki_limit = 10

# IP-based geolocation (shared by --here flag and the `here` plugin)
here_on_startup = false
geoip_endpoint = "https://ipapi.co/json/"
geoip_timeout_ms = 2000

[keymap]
zoom_in = ["i", "+"]
quit = ["q", "C-q"]
```

See `config.example.toml` for all options. The flat layout is scheduled for refactor into `[section]` form ([#47](https://github.com/Kohei-Wada/ttymap/issues/47)) before it grows further.

## Build

```bash
cargo build       # build.rs compiles proto/vector_tile.proto via protox
cargo test        # 154 tests
cargo clippy      # lint
```

## File locations

| Path | Content |
|------|---------|
| `~/.config/ttymap/config.toml` | Configuration |
| `~/.cache/ttymap/` | Disk tile cache |
| `~/.local/state/ttymap/ttymap.log` | Log file (auto-rotated at 1MB) |
