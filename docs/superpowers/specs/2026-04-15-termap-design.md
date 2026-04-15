# termap — Rust Terminal Map Viewer

## Overview

A terminal-based map viewer written in Rust that renders OpenStreetMap vector tiles as Braille/Unicode characters with ANSI colors. Vim-style navigation, extensible layer system for future data overlays (weather, routing, etc.).

## Goals

- Render OSM vector tiles in the terminal using Braille characters (2x4 pixel grid per character)
- Vim-style keyboard navigation with number prefixes, `gg`, `/` search, command mode
- Standalone CLI binary (no runtime dependencies), usable from vim via `:terminal termap`
- Library/binary split (`lib.rs` + `main.rs`) for future Neovim plugin potential
- Extensible layer architecture for overlaying additional data sources
- Mark mode for pinning points and measuring straight-line distance

## Non-Goals (Initial Scope)

- MBTiles / local tile support
- Mouse interaction
- ASCII fallback mode (Braille only)
- Routing engine / path finding
- TUI framework (ratatui etc.)

## Architecture

```
src/
  main.rs          CLI entry point (clap) -> App
  lib.rs           Public library interface
  app.rs           Main event loop (crossterm events -> render cycle)
  input.rs         Vim-style key handling (state machine)
  tile_source.rs   HTTP tile fetching + disk cache
  tile.rs          Protobuf decode -> layers/features
  renderer.rs      Features -> Canvas draw commands
  canvas.rs        Combines BrailleBuffer + LabelBuffer
  braille.rs       Pixel buffer -> Braille chars + ANSI color output
  label.rs         Label placement + collision detection (R-tree)
  styler.rs        JSON style -> draw parameters
  layer.rs         Layer trait definition + built-in OSM layer
  geo.rs           Coordinate transforms (lon/lat <-> tile coords)
  config.rs        Default configuration
```

## Component Design

### geo.rs — Coordinate Transforms

Mercator projection utilities:

- `ll2tile(lon, lat, zoom) -> TileCoord` — longitude/latitude to tile x/y at given zoom
- `tile2ll(x, y, zoom) -> LonLat` — tile coordinates back to lon/lat
- `normalize(LonLat) -> LonLat` — clamp to valid ranges (lon: -180..180, lat: -85.0511..85.0511)
- `base_zoom(zoom: f64) -> u32` — fractional zoom to integer tile zoom
- `tile_size_at_zoom(zoom: f64) -> f64` — pixel size of a tile at fractional zoom

### tile_source.rs — Tile Fetching

- Default source: `http://mapscii.me/{z}/{x}/{y}.pbf`
- Uses `reqwest::blocking` for HTTP requests
- Disk cache in platform cache dir (`directories` crate) at `~/.cache/termap/{z}/{x}-{y}.pbf`
- In-memory LRU cache (16 tiles)
- Gzip decompression via `flate2` when needed

### tile.rs — Tile Decoding

- Decode `.pbf` using `prost` (or `protobuf` crate) with Mapbox Vector Tile spec
- Extract layers, each containing features with:
  - Geometry type (Point, LineString, Polygon)
  - Coordinates in tile-local space (0..4096 extent)
  - Properties (name, type, etc.)
- Build R-tree (`rstar`) spatial index per layer for viewport culling

### styler.rs — Style Engine

- Parse Mapbox GL Style Spec subset from JSON (dark.json / bright.json)
- Resolve style for a given layer + feature: color, line-width, min/max zoom, type (line/fill/symbol)
- Color: hex string -> 256-color terminal code via closest match
- Zoom-dependent stops: pick value for current zoom level

### renderer.rs — Rendering Pipeline

Per frame:

1. Calculate visible tiles (3x3 grid around center)
2. Fetch tiles (cache-first)
3. For each tile, query R-tree for features in viewport
4. Draw in layer order: landuse, water, building, road, admin, labels
5. Non-label features drawn first, labels sorted by rank and drawn last

Feature drawing by type:
- **Line**: Bresenham rasterization with width support
- **Fill**: earcut triangulation -> filled triangles via scanline
- **Symbol/Label**: text placement with collision detection

### canvas.rs — Drawing Surface

Combines two buffers:
- `BrailleBuffer` — pixel-level drawing (lines, polygons)
- `LabelBuffer` — text labels with collision avoidance

`frame()` merges both into a single string of Braille chars + ANSI escape sequences.

### braille.rs — Braille Pixel Buffer

Each terminal character = 2x4 pixel grid mapped to Unicode Braille (U+2800..U+28FF):

```
Bit layout per character cell:
  [0x01] [0x08]
  [0x02] [0x10]
  [0x04] [0x20]
  [0x40] [0x80]
```

- `set_pixel(x, y, color)` — set bit + foreground color
- `frame()` — iterate buffer, emit Braille char + ANSI color codes per cell
- Foreground and background color buffers (256-color terminal palette)

### label.rs — Label Placement

- R-tree based collision detection
- For each label candidate position, check if bounding box overlaps existing labels
- Margin configurable per layer (e.g., POI labels need more spacing)
- Cluster fallback: if full label doesn't fit, try placing a marker icon instead

### input.rs — Vim-Style Input

State machine for key processing:

**Normal mode:**
| Key | Action |
|-----|--------|
| `h/j/k/l` | Pan left/down/up/right |
| `a` | Zoom in |
| `z` | Zoom out |
| `gg` | Zoom to world view (min zoom) |
| `{n}{motion}` | Repeat motion n times (e.g., `10j`) |
| `/` | Enter search mode |
| `:` | Enter command mode |
| `m` | Toggle mark mode (pin point A, then B) |
| `q` | Quit |
| `c` | Cycle through available styles |

**Search mode (`/`):**
- Text input for place name search
- Enter to search, Esc to cancel
- Uses Nominatim API for geocoding

**Command mode (`:`):**
| Command | Action |
|---------|--------|
| `:q` | Quit |
| `:zoom {n}` | Set zoom level |
| `:goto {lat} {lon}` | Jump to coordinates |
| `:center` | Show current center coordinates |
| `:marks` | List pinned marks |
| `:clearmarks` | Remove all marks |

**Number prefix accumulator:**
- Digits accumulate into a repeat count
- Next non-digit key triggers the action N times
- Resets on Esc or after execution

### layer.rs — Extensible Layer System

```rust
pub trait Layer {
    /// Unique identifier for this layer
    fn id(&self) -> &str;

    /// Fetch data relevant to the current viewport
    fn fetch(&mut self, bounds: &ViewBounds, zoom: f64) -> Result<()>;

    /// Draw layer contents onto the canvas
    fn draw(&self, canvas: &mut Canvas, bounds: &ViewBounds, zoom: f64);

    /// Whether this layer is currently enabled
    fn enabled(&self) -> bool;
}
```

Built-in layers:
- `OsmTileLayer` — the core map (tiles + style + labels)
- `MarkerLayer` — pinned points + distance line

Future layers (not in initial scope):
- `WeatherLayer` — overlay temperature/precipitation from API
- `RouteLayer` — routing polyline from OSRM/Valhalla API

### app.rs — Main Loop

```
loop {
    // 1. Read crossterm event (with timeout for non-blocking)
    // 2. Pass to input.rs state machine
    // 3. If state changed (pan/zoom/mark), trigger re-render
    // 4. renderer.draw(center, zoom) -> frame string
    // 5. Write frame to stdout
    // 6. Update status bar (coordinates, zoom, marks)
}
```

Terminal setup/teardown:
- Enable raw mode + alternate screen on start
- Disable on exit (including panic handler for clean restore)

### config.rs — Configuration

```rust
pub struct Config {
    pub source: String,           // "http://mapscii.me/"
    pub style_file: String,       // path to style JSON
    pub initial_lat: f64,         // 52.51298 (Berlin)
    pub initial_lon: f64,         // 13.42012
    pub initial_zoom: Option<f64>,
    pub max_zoom: f64,            // 18.0
    pub zoom_step: f64,           // 0.2
    pub cache_tiles: bool,        // true
    pub language: String,         // "en"
    pub label_margin: u16,        // 5
}
```

CLI args (clap):
- `--lat`, `--lon` — initial center
- `--zoom` — initial zoom
- `--style` — path to style JSON
- `--source` — tile server URL
- `--width`, `--height` — fixed terminal size override

### Mark Mode

1. Press `m` to enter mark mode
2. Current crosshair position is pinned as point A (displayed as `A` on map)
3. Press `m` again to pin point B
4. A line is drawn between A and B with the great-circle distance shown in the status bar
5. `m` again clears marks and starts over
6. `:clearmarks` removes all marks

Distance calculation: Haversine formula.

## Dependencies

| Crate | Purpose |
|-------|---------|
| `crossterm` | Terminal control, key/mouse events, raw mode |
| `clap` | CLI argument parsing |
| `reqwest` (blocking) | HTTP tile fetching |
| `flate2` | Gzip decompression |
| `prost` + `prost-types` | Protobuf decoding |
| `rstar` | R-tree spatial index |
| `earcutr` | Polygon triangulation (earcut) |
| `serde` + `serde_json` | Style JSON parsing |
| `directories` | Platform cache directory paths |

## Styles

Port the existing `dark.json` and `bright.json` from MapSCII. These are subsets of the Mapbox GL Style Spec containing:
- Layer visibility rules (min/max zoom)
- Paint properties (line-color, fill-color, text-color, line-width)
- Background color

## Status Bar

Bottom line of terminal showing:
- Current center coordinates
- Current zoom level
- Mark info (if marks are placed): "A: lat,lon  B: lat,lon  dist: 1.23km"
- Mode indicator: `-- NORMAL --`, `-- SEARCH --`, `-- COMMAND --`, `-- MARK --`

## Error Handling

- Network failures: show "offline" indicator, use cached tiles where available
- Tile decode failures: skip tile, render remaining tiles
- Terminal resize: re-render on SIGWINCH
- Panic: restore terminal state via drop guard
