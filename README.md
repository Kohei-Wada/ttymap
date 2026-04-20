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
├── main.rs           CLI entry + interactive-mode composition
├── lib.rs            crate root
├── app.rs            App struct, event loop, composition root
├── app_msg.rs        AppMsg enum + dispatch (the controller) + DispatchCtx
├── config.rs         TOML config + CLI overrides
├── logging.rs        XDG state log
│
├── commands/         one file per CLI subcommand (main.rs stays thin)
│   ├── mod.rs        Command enum (#[derive(Subcommand)]) + run() dispatch
│   └── snap.rs       `ttymap snap` / `snapshot` — headless single-frame renderer
│
├── focus.rs          FocusManager — event-driven focus transitions
├── painter.rs        MapPainter — plugins' world-space drawing API
├── theme.rs          UiTheme + runtime theme switch
├── keymap.rs         KeyBinding → AppMsg table + user overrides
├── geo.rs            Web Mercator, MapProjection, distance
├── color_palette.rs  xterm-256 color tables (ThemeId + DARK/BRIGHT)
│
├── input/            pure translators: raw event → Option<AppMsg>
│   ├── mod.rs
│   ├── keyboard.rs   focus-first routing + gg sequence + keymap fallback
│   └── mouse.rs      drag/scroll → AppMsg::Map(PanCells/ZoomAt)
│
├── map/              domain — viewport state + rendering pipeline
│   ├── action.rs     Action enum (discrete + mouse-continuous variants)
│   ├── state.rs      MapState, MapStateOptions, Viewport
│   ├── render/       tiles → MapFrame on a dedicated thread
│   │   ├── pipeline.rs   RenderPipeline
│   │   ├── thread.rs     RenderHandle
│   │   ├── renderer.rs   Feature[] → Canvas
│   │   ├── canvas.rs     Braille drawing primitives
│   │   ├── braille.rs    2×4 pixel buffer
│   │   ├── frame.rs      MapFrame DTO
│   │   ├── view.rs       Visible-tile math
│   │   ├── label.rs      R-tree label collision buffer
│   │   └── geom/         Bresenham, clipping
│   ├── styler/       Mapbox GL-style rules (dark / bright presets)
│   └── tile/         MVT fetch + cache + decode
│       ├── cache.rs      Memory + disk LRU
│       ├── decode.rs     Protobuf → DecodedTile
│       └── fetch/        TileClient trait + mapscii HTTP backend
│
├── plugin/           plugin API + built-in plugins
│   ├── mod.rs        Plugin trait, PluginCtx, PluginAction, PluginRegistry
│   ├── help.rs       help popup
│   ├── here/         IP-geolocation "jump to here" (headless, palette-only)
│   ├── search/       forward-geocode popup
│   └── wiki/         nearby-Wikipedia panel
│
├── shared/           cross-cutting utilities
│   ├── async_job.rs  fire-and-poll background job (reused by geocode/wiki/here)
│   ├── geoip.rs      IP-based lat/lon lookup (shared by --here and the here plugin)
│   ├── http/         user-agent-tagged reqwest wrapper
│   ├── nominatim.rs  forward + reverse geocoding
│   └── throttle.rs
│
└── ui/               terminal UI framework
    ├── mod.rs        UiState + draw() + workflow methods (open_palette, …)
    ├── action.rs     UiAction enum (UI-level commands, e.g. SetTheme)
    ├── map_view.rs   MapFrame ratatui adapter
    │
    ├── palette/      command palette (builtin coordinator — see mod.rs)
    │   ├── mod.rs    CommandPalette + PaletteOutcome
    │   ├── panel.rs  ratatui Table popup
    │   ├── state.rs  query buffer + substring filter
    │   └── provider/ universal picker backends
    │       ├── mod.rs     PaletteProvider trait + PaletteAction
    │       ├── command.rs default provider (actions + plugins + sub-modes)
    │       └── theme.rs   theme-picker sub-mode
    │
    └── overlay/      built-in, always-on map decorations
        ├── attribution.rs   © OpenStreetMap
        ├── scale_bar.rs     distance ruler
        └── info/            center/cursor/zoom/place readout
            ├── mod.rs
            └── service.rs   async reverse-geocoder
```

### Layering

- **`map/`** — domain state. Knows nothing about UI or plugins. `Action` carries every map-level mutation, including mouse-emitted continuous variants (`PanCells`, `ZoomAt`); plugin activation and UI state are separate concerns.
- **`app_msg.rs`** — the **controller**. One `AppMsg` enum that every input source (keyboard, mouse, plugins, async polling, future API / MCP / Lua) emits; one `dispatch(cmd, &mut DispatchCtx)` that routes it to the right domain method. The single state mutator in the app.
- **`input/`** — pure translators. `keyboard.rs` / `mouse.rs` turn raw events into `Option<AppMsg>` and return it to `app.rs`; they never call `dispatch` themselves and never touch domain state directly. Symmetric with async plugin polling.
- **`focus.rs`** — `FocusManager` driven by `FocusEvent`s (`PaletteOpened`, `PluginActivated(tag)`, …). Callers emit *what happened*; the manager decides the transition (wants_focus gating, auto-release, prev-slot restoration). All focus writes live here.
- **`ui/overlay/`** — identity decorations (info, attribution, scale bar). Always rendered; not plugin territory.
- **`ui/palette/`** — command palette. A **builtin coordinator**, not a `Plugin`. Plugins contribute functionality; palette aggregates over the plugin registry + keymap + theme to present a picker. Folding it into `Plugin` would widen `PluginCtx` to grant every plugin access to the registry and reduce the self-contained-widget contract to a naming convention. The asymmetry is deliberate — see `src/ui/palette/mod.rs` for the full rationale.
- **`plugin/`** — the plugin surface. Built-in plugins (search, help, wiki, here) implement the `Plugin` trait and register into the `PluginRegistry`. The keyboard handler dispatches by focus + activation-key lookup, never by plugin name. Plugins emit `AppMsg`s via `PluginAction::Run(msg)` and `pending_command()`; they never touch `FocusManager` or `MapState` directly.

### Input flow

```
raw event
  ↓ input layer (keyboard / mouse / async poll)
  ↓ Option<AppMsg>          ← pure translation, no state mutation
  ↓
app.rs: self.dispatch(cmd)
  ↓
app_msg::dispatch(cmd, &mut ctx)    ← single state mutator
  ↓
    AppMsg::Map(a)            → ctx.map.process_action(&a)
    AppMsg::Jump(loc)         → ctx.map.jump_to(loc)
    AppMsg::Ui(a)             → ctx.ui.apply(a, render_handle)
    AppMsg::ActivatePlugin    → ctx.ui.activate_plugin(tag, center)
    AppMsg::CycleFocus(fwd)   → ctx.ui.cycle_focus(fwd)
    AppMsg::OpenPalette       → ctx.ui.open_palette(keymap)
    AppMsg::Resize(cols,rows) → ctx.map.resize + render_handle.request_resize
```

The keyboard translator's decision tree:

```
key event
  ↓ keyboard.handle() → Option<AppMsg>:
    [1] focused surface delivery via ui.deliver_key() — consumes, runs, or passes through
    [2] Tab / Shift-Tab        → AppMsg::CycleFocus(forward)
    [3] `:`                    → AppMsg::OpenPalette
    [4] plugin activation key  → AppMsg::ActivatePlugin(tag)
    [5] keymap.resolve()       → whatever AppMsg the binding produces
       (with gg sequence state on the handler)
```

Mouse is similar:

```
mouse event
  ↓ mouse.handle(event, &mut ui) → Option<AppMsg>:
    search focused?       → None (ignored)
    drag (left)           → AppMsg::Map(Action::PanCells(dx, dy))
    scroll up / down      → AppMsg::Map(Action::ZoomAt { anchor_*, zoom_in })
    (cursor readout side effect on InfoOverlay always)
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

Rendering is decoupled from fetching. The render thread produces a `MapFrame` from the current `Viewport`; the main thread consumes it. Stale frames are fine — overlays reproject against the frame's own center/zoom.

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
    fn pending_command(&mut self) -> Option<AppMsg>;  // async-emitted message (e.g. Jump)
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
