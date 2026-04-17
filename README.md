# ttymap

Terminal-based map viewer. Renders [Mapbox Vector Tiles](https://github.com/mapbox/vector-tile-spec) as Unicode Braille characters with ANSI 256-color in your terminal.

Inspired by [mapscii](https://github.com/rastapasta/mapscii).

## Features

- **Braille rendering** — 2x4 pixel grid per terminal cell for high-resolution maps
- **Vim-style navigation** — `hjkl` pan, `a`/`z` zoom, `gg` world view
- **Mouse support** — drag to pan, scroll to zoom towards cursor
- **Location search** — `/` to search with autocomplete (Nominatim)
- **Wikipedia panel** — `i` to show nearby Wikipedia articles, Enter to jump
- **Place name display** — reverse geocoding shows current location
- **Scale bar** — distance indicator that updates with zoom level
- **Help popup** — `?` shows all keybindings
- **Configurable** — keybindings, initial position, language via TOML config
- **Context-sensitive footer** — shows available keys for current mode

## Usage

```bash
cargo run                                         # default position
cargo run -- --lat 35.68 --lon 139.76 --zoom 10   # Tokyo
cargo run -- --style styles/bright.json            # bright theme
ttymap clear-cache                                 # clear disk tile cache
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

**Search mode (`/`):**
| Key | Action |
|-----|--------|
| Type | Filter locations (autocomplete after 3 chars) |
| `Tab` | Accept completion |
| `Enter` | Execute search |
| `↑` `↓` / `Ctrl-N` `Ctrl-P` | Navigate results |
| `Ctrl-H` | Delete character |
| `Ctrl-U` | Clear query |
| `Esc` | Cancel |

**Wikipedia panel (`i`):**
| Key | Action |
|-----|--------|
| `Ctrl-J` `Ctrl-K` / `Ctrl-N` `Ctrl-P` | Navigate articles |
| `Enter` | Jump to article location |
| `i` | Close panel |

Keybindings are customizable via `~/.config/ttymap/config.toml`.

## Architecture

```
src/
  main.rs             CLI entry point + subcommands (clear-cache)
  app.rs              Event loop, input routing, geocode orchestration
  lib.rs              Module declarations

  core/               State management (no render/tile/UI dependency)
    state.rs          Core: center, zoom, process_action()
    config.rs         Config struct + TOML file loader
    keymap.rs         Key bindings, key notation parser
    input.rs          Action enum, InputHandler (keymap lookup)
    snapshot.rs       RenderRequest (DTO sent to render thread)

  ui/                 ratatui-based UI layer
    mod.rs            UiState (bundles all widgets)
    layout.rs         Screen layout + context-sensitive footer
    theme.rs          Color constants and widget helpers
    widget/
      map.rs          MapFrame ratatui Widget impl
      search.rs       Search: input, autocomplete, result selection
      info.rs         Coordinates, place name, scale bar overlay
      wiki.rs         Wikipedia article panel with jump
      help.rs         Keybinding help popup

  render/             Display pipeline
    pipeline.rs       Orchestrates tile fetch + draw (owns TileCache + Renderer)
    renderer.rs       Feature[] → pixel output (no cache knowledge)
    thread.rs         Background thread (calls pipeline, knows nothing else)
    canvas.rs         Drawing primitives, line/polygon clipping
    braille.rs        2x4 pixel Braille buffer → MapFrame
    frame.rs          MapCell, MapFrame data types
    label.rs          R-tree collision-free label placement

  tile/               Tile data management
    cache.rs          Memory + disk LRU cache, decode, prefetch strategy
    client.rs         Fixed worker pool (6 threads), HTTP fetch only
    queue.rs          Generic priority queue (pluggable sort)
    decode.rs         MVT protobuf → DecodedTile with R-tree spatial index
    view.rs           Visible tile calculation (pure math)

  nominatim.rs        Nominatim API client (forward + reverse geocoding)
  geocode.rs          Async geocoding wrapper (search, complete, reverse)
  wikipedia.rs        Wikipedia API client (geosearch + extracts)
  styler.rs           Mapbox GL style JSON → color/filter rules
  geo.rs              Web Mercator math, distance, scale bar
  color.rs            Hex → RGB → xterm-256 conversion
  logging.rs          File logger (~/.local/state/ttymap/)
```

### Data flow

```
┌──────────────┐  RenderRequest   ┌────────────────────────┐
│  Main thread │ ──────────────→  │  Render thread         │
│              │                  │  RenderPipeline        │
│  App         │  MapFrame        │   TileCache → Renderer │
│   ├ Core     │ ←──────────────  │                        │
│   ├ Input    │                  └──────────┬─────────────┘
│   ├ Geocoder │                             │ cache miss
│   └ UiState  │                  ┌──────────▼─────────────┐
│     ├ Search │                  │  Worker pool (6)       │
│     ├ Info   │                  │  HTTP fetch → raw bytes│
│     ├ Wiki   │                  └────────────────────────┘
│     └ Help   │
│              │   Nominatim API
│  ratatui     │ ←─────────────── Geocoder (background thread)
│  Terminal    │
│              │   Wikipedia API
│              │ ←─────────────── WikipediaClient (background thread)
└──────────────┘
```

### Dependency direction

```
app → core, ui, render/thread, geocode, wikipedia
ui/layout → ui/widget/*, ui/theme
render/thread → render/pipeline
render/pipeline → render/renderer, tile/cache
render/renderer → render/canvas (no tile dependency)
tile/cache → tile/client, tile/decode
tile/client → tile/queue (HTTP fetch only)
geocode → nominatim
core → geo (no render, tile, or UI dependency)
```

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
cargo test        # 109 tests
cargo clippy      # lint
```

## File locations

| Path | Content |
|------|---------|
| `~/.config/ttymap/config.toml` | Configuration |
| `~/.cache/ttymap/` | Disk tile cache |
| `~/.local/state/ttymap/ttymap.log` | Log file (auto-rotated at 1MB) |
