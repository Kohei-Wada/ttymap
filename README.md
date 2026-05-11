# ttymap

[![CI](https://github.com/Kohei-Wada/ttymap/actions/workflows/ci.yml/badge.svg)](https://github.com/Kohei-Wada/ttymap/actions/workflows/ci.yml)

**Terminal-native scriptable globe.** Mapbox Vector Tiles rendered as Unicode Braille with ANSI 256-color, on top of a first-class Lua plugin runtime — real-time data overlays, animated camera tours, scientific computations, and a small "scriptable scenes" engine (animation + coroutine scheduler) that turns ttymap into a programmable canvas for spatial data.

> ⚠️ **Work in progress.** Stable enough to use daily, but APIs (CLI flags, Lua surface, config schema) may change without notice during the WIP phase.

https://github.com/user-attachments/assets/63c6415f-9d3e-4d0c-957c-92ca5f326e43

<details>
<summary>Screenshots</summary>

Default view with help panel and satellite tracker:

![ttymap default view with help panel and satellite tracker](assets/ttymap-default.png)

Bright theme:

![ttymap bright theme](assets/ttymap-bright.png)

Tokyo zoomed in with the wiki panel open:

![ttymap zoomed-in Tokyo with wiki panel](assets/ttymap-zoom-wiki.png)

</details>

## What you get

- **Map core** — MVT decoding, Mercator projection, Braille rendering at 2×4 sub-pixels per cell, ANSI 256-color, per-feature spatial indexing (R-tree). The original `mapscii`-style terminal viewer is here as one component.
- **Vim-style navigation** — `hjkl` pan, `b`/`w` fast pan, `C-u`/`C-d` half-screen pan, `a`/`z` zoom, `gg` world view, `0` reset, mouse drag + scroll.
- **Command palette** — `:` for actions, `/` for Nominatim location search.
- **Lua plugin runtime** — every in-tree feature is a Lua script under `runtime/lua/plugin/`; user plugins go in `~/.config/ttymap/lua/plugin/` (Neovim-style stem-dedup).
- **Lua-based config** — `~/.config/ttymap/init.lua`, with conditional / computed values (the killer feature over TOML).
- **Scriptable scenes** — `ttymap.animation.fly_to` (frame-based pan/zoom) + `ttymap.director` (coroutine scheduler with `fly` / `wait` / `tween` primitives). Plugins can choreograph multi-step camera + overlay sequences as procedural Lua.
- **Headless snapshot** — `ttymap snap …` writes the current view as ANSI text for dashboards / cron / pipes.

## Bundled plugins

17 plugins ship with the runtime. Each is a reference for one shape — toggleable overlay, palette one-shot, side panel, palette provider, and a quick game. The list is the answer to "what is ttymap actually for". Plus one bundled lib (`ttymap.notify`) that renders `ttymap.notify(...)` toasts top-left for every plugin.

| Plugin | What it does |
| --- | --- |
| `aircraft` | Live aircraft markers from the OpenSky public ADS-B feed; sidebar list with altitude / speed; Enter centers the map. |
| `antipode` | `:` palette → fly to the diametrically opposite point on the sphere. "What's directly below me through Earth?" |
| `attribution` | Always-on © OpenStreetMap chrome (legal hygiene for a map renderer). |
| `center` | Crosshair at the map center — handy when fast-panning. |
| `export` | `:` palette → dump current frame as ANSI to disk; pipe to `cat` or share as a snapshot. |
| `geo_quiz` | "Find this city before time runs out" — a target pops up, you have ~30 s to pan / zoom so the map center lands as close as possible. Submit with Enter; the camera flies out to a view that frames both your guess and the real city with ◎ markers + a connecting line. Score is cumulative km error (golf-style, lower is better). Easy mode shows the country, hard mode doesn't. |
| `help` | `?` opens a live keymap cheatsheet derived from the active keymap + plugin palette entries. |
| `here` | `:` palette → IP-geolocate then animate the camera home. |
| `info` | Top-right readouts: center / cursor / zoom / pan speed + bearing / solar time / distance from your geoip home / reverse-geocoded place name. |
| `ping_simulation` | "Cyber-attack visualisation"-style growing lines pinging between cities — reference for animated polyline overlays. |
| `quake` | USGS magnitude-2.5+ earthquakes from the past 24 hours; colored markers + sidebar list, auto-jumps to the highest-magnitude event. |
| `satellite` | Multi-sat tracker (ISS, Hubble, …) with TLE fetch from CelesTrak and SGP4 propagation in Lua. |
| `scalebar` | Bottom-right scale ruler that adapts to current zoom. |
| `search` | Forward geocoding via Nominatim, palette-provider style with debounced input. |
| `terminator` | Day/night boundary as a polyline; ☀ subsolar / ☾ antisolar markers; updates from the wall clock. |
| `traceroute` | `r` → host prompt → animated polyline grows hop-by-hop along the route as `traceroute(8)` resolves each next-hop IP via ip-api.com; sidebar lists hops (Enter to fly to a router), per-hop colour gradient, consecutive `*` runs collapsed in the panel. |
| `travel` | Curated multi-country itineraries (Japan + Italy out of the box) with an animated tour: pre-overview → stop loop → post-overview, driven by `ttymap.director`. |
| `wiki` | Wikipedia geosearch — markers + side panel of nearby articles; Enter opens an extract paragraph. |

Each lives as a single `*.lua` (or directory with `init.lua`) under [`ttymap-tui/runtime/lua/plugin/`](ttymap-tui/runtime/lua/plugin/) — readable as a tutorial for writing your own.

## Scriptable scenes

The animation + director libs are the bit that takes ttymap past "interactive viewer" into "programmable globe". A complete tour script:

```lua
local director = require "ttymap.director"

director.run(function()
    ttymap.notify("Starting tour")
    director.fly(139.69, 35.69, 10)            -- yields until the camera arrives in Tokyo
    director.wait(120)                           -- park there for ~2s at 60fps
    director.fly(-74.00, 40.71, 10)            -- glide to New York
    director.wait(120)
    director.fly(2.35,   48.86, 10)            -- glide to Paris
    director.wait(120)
end, {
    on_cancel = function() ttymap.notify("Tour cancelled") end,
})
```

`director.run` registers a coroutine; `director.fly` / `director.wait` / `director.tween` yield until their condition is met. Multiple `run` calls execute in parallel under one `on_tick` driver. Manual user pan / zoom during a `fly` cancels the script via the animation lib's tolerance check — same hook used to bail a tour cleanly.

The travel plugin's pre / stop-loop / post phases are one such script. ping_simulation's per-ping animations are another:

https://github.com/user-attachments/assets/53790bbf-053c-4515-88b6-405b195b82a3

https://github.com/user-attachments/assets/eef66370-2981-475c-9678-39ecf7113bc3

(Regenerate locally with `vhs vhs/travel.tape` / `vhs vhs/ping_simulation.tape`.)

See [`docs/lua-architecture.md`](docs/lua-architecture.md) for the full plugin authoring surface — `ttymap.api.*`, plugin shapes, drain pattern, runtime path resolution, config chain.

## Install

```bash
git clone https://github.com/Kohei-Wada/ttymap
cd ttymap
make install
```

Installs `~/.cargo/bin/ttymap` + `~/.local/share/ttymap/` (bundled runtime). Single-user, no root. `cargo install` alone fails fast with a "did you `make install`?" message because the runtime needs to be placed.

## Usage

**Interactive:**

```bash
ttymap                                       # default position
ttymap --lat 35.68 --lon 139.76 --zoom 10    # Tokyo
ttymap --style bright                        # bright theme
```

For "jump to my current location" use the bundled `here` plugin from the `:` palette — it does an IP-geolocation lookup on demand and flies the camera over.

**Headless snapshot:**

```bash
ttymap snap --lat 35.68 --lon 139.76 --zoom 12               # → stdout
ttymap snap --lat 35.68 --lon 139.76 --zoom 12 -o tokyo.ans  # → file
```

`snap` emits raw xterm-256 ANSI; `cat` the file in any compatible terminal or pipe to `less -R`.

Press `?` in interactive mode for the live keymap cheatsheet.

## Build

```bash
cargo build       # build.rs compiles proto/vector_tile.proto via protox
cargo test
cargo clippy
```

Rust 2024 edition. The build uses `protox` so no system `protoc` is needed.

## Documentation

- **[docs/architecture.md](docs/architecture.md)** — system layout, threads, message + render flow, focus model, concurrency.
- **[docs/configuration.md](docs/configuration.md)** — `init.lua` reference, runtime path resolution, file locations.
- **[docs/lua-architecture.md](docs/lua-architecture.md)** — plugin authoring guide: `ttymap.api.*` surface, shared libraries (`ttymap.fmt` / `.sidebar` / `.animation` / `.director` / `.location` / `.geo`), plugin shapes, dispatcher semantics.
- **[docs/design.md](docs/design.md)** — load-bearing design decisions (UserCommand vs direct call, controller split, Drop-based cleanup).

## Origin

ttymap started as a Rust port of [mapscii](https://github.com/rastapasta/mapscii) — same Braille rendering, same MVT pipeline, same vim-style nav. The Lua plugin runtime and the scriptable-scenes layer (animation + director) grew on top once the engine was solid; the result is a different category of tool now, but mapscii is the seed and deserves the credit.

If you want the **minimum** mapscii-equivalent experience, ttymap with the bundled runtime gives you that out of the box. If you want to tour cities on a script, paint scientific overlays, or wire a live data feed onto the map, the plugin runtime is there for it.

## Roadmap

Principles:

- **Core stays lean.** A map viewer + plugin runtime, not a GIS platform. Tile rendering, projection, navigation, plugin host. Anything domain-specific is a Lua plugin.
- **Plugin-first.** Every built-in is a Lua script — the bridge dogfoods itself.
- **Boring where it matters.** Stable protocols (MVT, OSM), predictable resource use, `cargo install` ships a single binary.

Short-term work + plugin candidates are tracked in [GitHub issues](https://github.com/Kohei-Wada/ttymap/issues). Notable in-flight: alternate tile backends ([#30](https://github.com/Kohei-Wada/ttymap/issues/30) MBTiles, [#31](https://github.com/Kohei-Wada/ttymap/issues/31) PMTiles), persistent plugin storage ([#194](https://github.com/Kohei-Wada/ttymap/issues/194)), declarative plugin SDK ([#217](https://github.com/Kohei-Wada/ttymap/issues/217)).

## Contributing

PRs and issues are very welcome. See [CONTRIBUTING.md](CONTRIBUTING.md) for the dev workflow and a heads-up about breaking changes during this WIP phase.

## Platform support

CI runs on Linux, macOS, and Windows for every push (see [`.github/workflows/ci.yml`](.github/workflows/ci.yml)). Linux is the primary daily-driver target; macOS and Windows are smoke-tested but get less interactive testing.

### Troubleshooting

- **Windows: install via `cargo` directly** — `make install` assumes a POSIX shell. Until the Makefile grows a Windows path, build with `cargo build --release` and copy `target/release/ttymap.exe` plus the `ttymap-tui/runtime/` directory to wherever you want them. Set `TTYMAP_RUNTIME=path\to\runtime` if you don't place it under the platform-default data dir (`%APPDATA%\ttymap\runtime`).
- **Windows: use Windows Terminal, not legacy ConHost** — Braille glyphs and xterm-256 colors need a font with full Braille coverage (Cascadia Mono works) and a terminal that respects 256-color ANSI. Legacy ConHost (the default `cmd.exe` window pre-Windows 11) renders Braille as boxes and clamps to 16 colors.
- **macOS: tile cache & exported frames live under `~/Library/Caches/ttymap` and `~/Library/Application Support/ttymap`** — different from Linux's XDG paths. The `directories` crate handles this transparently; only relevant if you script around the cache.
- **Mouse drag/scroll on Windows** — works in Windows Terminal; some third-party emulators don't forward mouse events. Toggle off via config if your terminal traps them.

## License

Dual-licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or <http://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or <https://opensource.org/licenses/MIT>)

at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in the work by you, as defined in the Apache-2.0 license, shall be dual-licensed as above, without any additional terms or conditions.
