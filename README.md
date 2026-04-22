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
- **Component API** — built-in plugins (search, wiki, here, palette, help) are `Component`s on a focus stack; external plugins will use the same trait

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
├── main.rs              CLI entry + interactive-mode composition
├── lib.rs               crate root
├── logging.rs           XDG state log
├── config.rs            TOML config (sectioned) + CLI overrides
├── keymap.rs            KeyBinding → AppMsg table + user overrides
├── geo.rs               Web Mercator, projection, distance
├── painter.rs           MapPainter — components' world-space drawing API
│
├── theme/                colour palette + ratatui adapter
│   ├── mod.rs            ThemeId + re-exports
│   ├── palette.rs        ColorPalette struct + DARK / BRIGHT consts (xterm-256)
│   └── ui.rs             UiTheme (ratatui style adapter)
│
├── app/                 App struct + event loop + message dispatch
│   ├── mod.rs           App::new / run / dispatch — single side-effect boundary
│   └── msg.rs           AppMsg enum (Map / Jump / SetTheme / CycleFocus / …)
│
├── commands/            one file per CLI subcommand (main.rs stays thin)
│   ├── mod.rs           Command enum + run() dispatch
│   └── snap.rs          `ttymap snap` / `snapshot` — headless single-frame renderer
│
├── compositor/          helix-inspired focus / modal stack
│   ├── mod.rs           Component trait, Compositor, Registrar, Task, Activation
│   ├── base.rs          BaseLayer — keymap + activation dispatch + gg sequence
│   └── window.rs        Window (event-side) + RenderWindow (render-side, owns UiTheme)
│
├── widget/              ratatui-agnostic render vocabulary
│   ├── geom.rs          Rect / Size
│   ├── style.rs         StyleKind (Body / Accent / Muted / Selected / Link / …)
│   ├── text.rs          Line / Span
│   └── paragraph.rs, list.rs, table.rs
│
├── palette/             `:`-triggered universal picker (itself a Component)
│   ├── mod.rs           CommandPalette + install(&mut Registrar)
│   ├── panel.rs         popup layout
│   └── provider/        default provider + theme sub-mode
│
├── plugin/              built-in plugins — each exposes `pub fn register(…, &mut Registrar)`
│   ├── help/            help popup
│   ├── here/            IP-geolocation "jump to here" (headless Task)
│   ├── search/          forward-geocode popup (Nominatim)
│   └── wiki/            nearby Wikipedia panel
│
├── map/                 domain — viewport state + rendering pipeline
│   ├── state.rs, action.rs, mod.rs
│   ├── render/          tiles → MapFrame on a dedicated thread
│   │   ├── pipeline.rs, thread.rs, renderer.rs
│   │   ├── canvas.rs, braille.rs, frame.rs
│   │   └── view.rs, label.rs, geom/, earcut_worker.rs
│   ├── styler/          Mapbox GL-style rules (dark / bright presets)
│   └── tile/            MVT fetch + cache + decode
│       ├── cache.rs         Memory (configurable LRU) + optional disk
│       ├── decode.rs        Protobuf → DecodedTile
│       └── fetch/           TileClient trait + mapscii backend + priority queue
│
├── shared/              cross-cutting utilities
│   ├── async_job.rs     fire-and-poll background job
│   ├── geoip.rs         IP-based lat/lon lookup
│   ├── http/            user-agent-tagged reqwest wrapper
│   ├── nominatim.rs     forward + reverse geocoding
│   └── throttle.rs
│
└── ui/                  non-modal UI framework
    ├── mod.rs           UiState + draw() — owns overlay + last MapFrame
    ├── map_view.rs      MapFrame → ratatui widget
    ├── mouse.rs         MouseAdapter: MouseEvent → Vec<AppMsg>
    └── overlay/         always-on map decorations
        ├── attribution.rs   © OpenStreetMap
        ├── scale_bar.rs     distance ruler
        ├── info/            center / cursor / zoom / place readout
        └── manager.rs       OverlayManager
```

### Layering

- **`map/`** — domain. Knows nothing about UI, plugins, or focus. `Action` carries every map-level mutation, including mouse-continuous variants (`PanCells`, `ZoomAt`).
- **`app/`** — the **controller**. `AppMsg` (in `app/msg.rs`) is the closed enum every input source (keymap, palette, compositor components, mouse adapter, async tasks) emits; `App::dispatch` in `app/mod.rs` is the sole place that executes them. Command pattern with `App` as the Receiver — see [`docs/design.md`](docs/design.md) for the AppMsg-vs-direct-call judgment rules.
- **`compositor/`** — focus and modal state. A stack of `Component`s; the top is focused. No `is_visible` / `activate` / `deactivate` contract — presence on the stack *is* the lifecycle. `Tab` / `Shift-Tab` cycle focus (framework-reserved, intercepted before any component sees them).
- **`ui/mouse.rs`** — pure adapter. `MouseEvent → Vec<AppMsg>` (`CursorMoved` on every event; drag → `Map(PanCells)`; scroll → `Map(ZoomAt)`). No state mutation.
- **`ui/`** (non-mouse) — non-modal chrome: map view, always-on overlays (info, attribution, scale bar), and `draw()` which forwards focused-surface rendering to the Compositor.
- **`palette/`** — `:`-triggered universal picker. Itself a `Component`; its provider table is harvested from the `Registrar` at boot so plugins' palette entries appear automatically. Palette installs last so it sees everyone else's entries.
- **`plugin/`** — built-in plugins. Each module exposes `pub fn register(…, &mut Registrar)`; the compositor never names a concrete plugin type. Plugins implement `Component` (visual surfaces) or `Task` (headless async jobs); they emit `AppMsg` via `win.emit(msg)`.
- **`widget/`** — ratatui-agnostic render vocabulary. Plugins describe *what* to draw (`widget::Paragraph`, `Line`, `StyleKind::Accent`) and `RenderWindow` translates it to ratatui. Plugins never import ratatui or `UiTheme` directly.

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
  ui::draw(f, &ui, &compositor, &theme, &ctx):
    1. map_view renders the latest MapFrame
    2. MapPainter set up; compositor.paint_on_map(painter)
       — components paint world-space primitives (wiki markers, …)
    3. always-on overlays (info, attribution, scale_bar) stamp their rects
    4. compositor.render(f, area, theme, ctx)
       — every Component on the stack drawn bottom-up
    5. footer hints from the focused component
```

### Focus model

Focus is a `focused_idx` into the Compositor stack, **decoupled from stack position**. Pushing a modal puts focus on it; `Tab` moves focus back to the base layer without popping the modal (the old `Focus::Background` behaviour). Stack order never changes through cycling — only which component receives keys first.

Dedup is by `Any::type_id`: pressing an activation key while the plugin is already on the stack focuses the existing instance instead of stacking a duplicate. A plugin author cannot forget to opt in — the concrete type *is* the identity.

### Plugin API (built-ins + future external plugins)

```rust
trait Component: Any {
    /// Handle one key event. Communicate intent via win.*.
    fn handle_event(&mut self, event: KeyEvent, win: &mut Window);

    /// Paint into win.area(). Theme + ratatui Frame reached through win.
    fn render(&self, win: &mut RenderWindow);

    /// World-space primitives on the map (e.g. wiki markers). Default no-op.
    fn paint_on_map(&self, _p: &mut MapPainter<'_>) {}

    /// Tick-driven async polling. Default no-op.
    fn poll(&mut self, _win: &mut Window) {}

    /// Footer hints shown while this component is on top.
    fn footer_hints(&self) -> Vec<(&'static str, &'static str)> { vec![] }
}

// Headless plugin — no UI, no focus. The `here` geoip lookup uses this.
trait Task {
    fn poll(&mut self) -> Vec<AppMsg>;
}
```

Each plugin module exposes `pub fn register(…, &mut Registrar)` which contributes some mix of: an `Activation` (key → component factory), a `PaletteEntry` (label for `:`), and/or a `Task`.

Components never import ratatui or `UiTheme` directly — everything flows through `Window` (event side) and `RenderWindow` (render side). That containment lets the host own visual invariants (focused border colour, panel layout) and prevents a misbehaving plugin from painting outside its rect. `MapPainter` similarly hides projection and buffer behind primitives like `point(ll, glyph, fg)`.

### Concurrency

| Thread | Responsibility |
|--------|----------------|
| main | event loop, compositor, UI state, terminal draw |
| render | MapFrame generation (tile fetch + draw) |
| tile fetch | HTTP workers with priority queue |
| async jobs | Nominatim / Wikipedia / geoip (fire-and-poll via `shared::async_job`) |

mpsc channels connect the threads; the main thread never blocks on I/O.

## Roadmap

ttymap aims to be a **modern Rust replacement for mapscii** — still a terminal map viewer at heart, but with a first-class plugin story so the interesting overlays (planes, ships, weather, …) live outside the core.

### Principles

- **Core stays lean.** A map viewer, not a GIS platform. The core handles tiles, projection, rendering, navigation, and a small palette of general-purpose built-ins. Anything domain-specific is a plugin.
- **Plugin-first.** Every built-in (search, wiki, here, help) uses the same trait external plugins will. Built-ins dogfood the API.
- **Boring where it matters.** Stable protocols (MVT, OSM, TOML), predictable resource use, `cargo install` ships a single binary.

### Short-term

- **Tile backends** ([#30](https://github.com/Kohei-Wada/ttymap/issues/30) MBTiles, [#31](https://github.com/Kohei-Wada/ttymap/issues/31) PMTiles) — offline and CDN-friendly serving. Today the only backend is `mapscii.me`.
- **Error handling policy** ([#17](https://github.com/Kohei-Wada/ttymap/issues/17)) — normalize how soft errors (network, parse) surface.

### Mid-term — external plugin architecture

The current `Component` trait is in-process Rust. To let contributors ship plugins without touching this repo or matching an unstable ABI, the plan is:

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
- **Write a plugin** — the simplest real example is `src/plugin/here/mod.rs` (no UI, one palette command, async background job via `Task`). `src/plugin/search/mod.rs` is a good starting point for a modal `Component` with its own keymap. Once the subprocess architecture lands, plugins can live in their own repos.
- **Fix a bug or clean something up** — PRs welcome. The pre-commit hook runs tests, clippy, and rustfmt; follow its lead.

Issues on GitHub carry the current opinion of what's easy, what's hard, and what's deferred. Skim them before designing.

## Configuration

Config file: `~/.config/ttymap/config.toml`

```toml
[map]
lat = 35.6828
lon = 139.7595
zoom = 10.0

[render]
language = "ja"

[wiki]
limit = 10

# IP-based geolocation (shared by --here flag and the `here` plugin)
[geoip]
on_startup = false
endpoint = "https://ipapi.co/json/"
timeout_ms = 2000

[keymap]
zoom_in = ["i", "+"]
quit = ["q", "C-q"]
```

See `config.example.toml` for all options. Every section and field is optional; omitted values fall back to built-in defaults.

## Build

```bash
cargo build       # build.rs compiles proto/vector_tile.proto via protox
cargo test
cargo clippy
```

## File locations

| Path | Content |
|------|---------|
| `~/.config/ttymap/config.toml` | Configuration |
| `~/.cache/ttymap/` | Disk tile cache |
| `~/.local/state/ttymap/ttymap.log` | Log file (auto-rotated at 1MB) |
