# ttymap

Terminal-based map viewer. Renders [Mapbox Vector Tiles](https://github.com/mapbox/vector-tile-spec) as Unicode Braille characters with ANSI 256-color in your terminal.

Inspired by [mapscii](https://github.com/rastapasta/mapscii).

## Features

- **Braille rendering** — 2x4 pixel grid per terminal cell for high-resolution maps
- **Vim-style navigation** — `hjkl` pan, `a`/`z` zoom, `gg` world view
- **Mouse support** — drag to pan, scroll to zoom towards cursor
- **Location search** — `/` to search with autocomplete (Nominatim)
- **Wikipedia panel** — `i` to show nearby Wikipedia articles, Enter to jump
- **Cursor readout** — live lat/lon under the mouse cursor
- **Place name display** — reverse geocoding shows current location
- **Scale bar + attribution** — always on screen
- **Help popup** — `?` shows all keybindings
- **Configurable** — keybindings, initial position, language via TOML config
- **Plugin API** — built-in features (search, help, wiki) use the same `Plugin` trait external plugins will

## Usage

```bash
cargo run                                         # default position
cargo run -- --lat 35.68 --lon 139.76 --zoom 10   # Tokyo
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
| `/` | Search location (autocomplete) |
| `i` | Toggle Wikipedia panel |
| `?` | Toggle help |
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
├── palette.rs        xterm-256 theme tables
│
├── core/             domain — map state + commands
│   ├── action.rs     Action enum (map-level only)
│   └── state.rs      Core, CoreOptions, RenderRequest
│
├── plugin/           plugin API + built-in plugins (search, help, wiki)
│   ├── mod.rs        Plugin trait, PluginCtx, PluginAction, PluginRegistry
│   ├── help.rs
│   ├── search/
│   └── wiki/
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
│   ├── http/
│   ├── nominatim.rs
│   └── throttle.rs
│
└── ui/               terminal UI framework
    ├── mod.rs        UiState + draw()
    ├── focus.rs      Focus enum (Map | Plugin(tag))
    ├── map_view.rs   MapFrame ratatui adapter
    ├── painter.rs    MapPainter — plugins' world-space drawing API
    ├── theme.rs      UI color set
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
- **`plugin/`** — the plugin surface. Built-in plugins (search, help, wiki) implement the `Plugin` trait and are registered into the same `PluginRegistry` that will host external plugins. The keyboard handler dispatches by focus + activation-key lookup, never by plugin name.

### Input flow

```
key event
  ↓ keyboard.handle():
    [1] focused plugin sees the key first
    [2] registry activation lookup (plugins own their activation keys)
    [3] keymap.resolve(code, mods) → Action
    [4] core.process_action(&action)
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
    fn activation_keys(&self) -> Vec<&'static str>;
    fn activate(&mut self, ctx: &mut PluginCtx);
    fn deactivate(&mut self);
    fn handle_key(&mut self, code, mods, ctx) -> PluginAction;
    fn poll(&mut self) -> bool;
    fn render(&self, f, area, theme);              // focused panel
    fn footer_hints(&self) -> Vec<(&str, &str)>;
    fn paint_on_map(&self, p: &mut MapPainter);    // world-space primitives
}
```

All methods except `tag` and `handle_key` have defaults, so a passive
data-only plugin (e.g. a map-marker feed) can implement just `tag` +
`paint_on_map` and skip the UI-heavy parts.

`MapPainter` hides projection, buffer, and theme behind primitives like `point(ll, glyph, fg)` — plugins never compute screen coordinates themselves.

### Concurrency

| Thread | Responsibility |
|--------|----------------|
| main | event loop, UI state, terminal draw |
| render | MapFrame generation (tile fetch + draw) |
| tile fetch | HTTP workers with priority queue |
| geocode | Nominatim / Wikipedia calls |

mpsc channels connect the threads; the main thread never blocks on I/O.

## Configuration

Config file: `~/.config/ttymap/config.toml`

```toml
language = "ja"
lat = 35.6828
lon = 139.7595
zoom = 10.0
wiki_limit = 10

[keymap]
zoom_in = ["i", "+"]
quit = ["q", "C-q"]
```

See `config.example.toml` for all options.

## Build

```bash
cargo build       # build.rs compiles proto/vector_tile.proto via protox
cargo test        # 144 tests
cargo clippy      # lint
```

## File locations

| Path | Content |
|------|---------|
| `~/.config/ttymap/config.toml` | Configuration |
| `~/.cache/ttymap/` | Disk tile cache |
| `~/.local/state/ttymap/ttymap.log` | Log file (auto-rotated at 1MB) |
