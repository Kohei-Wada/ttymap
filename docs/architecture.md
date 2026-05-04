# Architecture

This is the system-level reference: module layout, threads, message flow,
render flow, focus model. For plugin authoring see
[lua-architecture.md](lua-architecture.md). For load-bearing design
decisions and trade-offs see [design.md](design.md).

## Source tree

The codebase is split into four logical layers, made visible at the
directory level (issue #212):

- **Foundation** (top-level files): vocabulary types and pure data
  (`command.rs`, `geo.rs`, `config.rs`, `logging.rs`, `theme/` data
  parts).
- **`core/`** — engine. State that mutates in response to commands,
  rendering pipeline, focus stack, input translation. **Ratatui-free
  at the import-graph level.**
- **`front/`** — UI / IO shell. Ratatui adapter, palette picker,
  CLI subcommands. Imports from foundation + `core/` only; never
  imported FROM `core/`.
- **`app/`** — loop driver above core. Owns the ratatui terminal,
  drains the bus, forwards commands to core::Dispatcher.
- **`lua/`** — Lua bridge. Spans both layers (plugins implement
  core's `Component` trait but draw via front's `MapApi`).
- **`shared/`** — common utilities (http, geoip).

```
src/
├── main.rs              CLI entry + interactive / snap dispatch
├── lib.rs               crate root + layering doc
├── logging.rs           XDG state log (--log flag)            (foundation)
├── config.rs            Config struct + Default impls         (foundation)
├── geo.rs               Web Mercator, lon/lat ↔ tile coords   (foundation)
├── command.rs           UserCommand enum — GoF Command vocab  (foundation)
│
├── theme/               (foundation: data only — no ratatui)
│   ├── mod.rs           ThemeId
│   └── palette.rs       ColorPalette + DARK / BRIGHT consts (xterm-256 indices)
│
├── core/                (engine — ratatui-free)
│   ├── dispatcher.rs    GoF Receiver for UserCommand
│   ├── overlay.rs       per-frame overlay sink + redraw throttle
│   ├── sidebar.rs       sidebar visibility policy
│   │
│   ├── compositor/      helix-style focus / modal stack
│   │   ├── mod.rs       Compositor + Component trait + Placement (Floating / Sidebar)
│   │   ├── base.rs      BaseLayer — keymap + activation table + count prefixes (5j etc.)
│   │   ├── op.rs        Op enum — single drain vocabulary (Push / Close / Command)
│   │   ├── window.rs    Window (event-side, queues ops) + RenderWindow (render-side)
│   │   ├── render.rs    paint(...) — render orchestration walking the focus stack
│   │   ├── sidebar.rs   left-rail layout (up to 3 Sidebar cards, equal vertical split)
│   │   └── map_api.rs   MapApi — world-space + screen-space draw primitives for Lua tick
│   │
│   ├── input/           input subsystem
│   │   ├── thread.rs    producer thread blocking on crossterm::event::read
│   │   ├── keymap.rs    KeyMap table + KeybindingOverrides (init.lua + Lua keymap.set)
│   │   └── mouse.rs     MouseAdapter — MouseEvent → UserCommand (drag pan, scroll zoom-at)
│   │
│   └── map/             domain — viewport state + render pipeline + tile + styler
│       ├── mod.rs, state.rs, action.rs
│       ├── render/      tiles → MapFrame on a dedicated thread
│       │   ├── pipeline.rs, thread.rs, renderer.rs
│       │   ├── canvas.rs, braille.rs, frame.rs, frame_widget.rs, view.rs
│       │   ├── label.rs, overlay.rs, polygon.rs, project.rs, geom/
│       │   └── earcut_worker.rs, panic_silence.rs
│       ├── styler/      Mapbox GL-style rules; theme swaps ColorPalette only
│       └── tile/        MVT fetch + cache + decode
│           ├── cache.rs, decoder.rs, disk.rs, key.rs, property.rs
│           ├── decode/  Protobuf → DecodedTile (geometry / tags / decompress)
│           └── fetch/   TileFetcher + FetchLane + http + disk_cached decorator
│
├── front/               (UI / IO shell — ratatui-aware)
│   ├── theme/           ratatui adapter (UiTheme + StyleKind)
│   ├── palette/         `:`-triggered universal picker (a Component)
│   │   ├── mod.rs       PaletteComponent ephemeral state
│   │   ├── action.rs    PaletteAction (close / submit / SwitchProvider / …)
│   │   ├── panel.rs     popup layout
│   │   └── provider/    sync (OnEachKey filter) + async (Debounced + poll + spinner)
│   └── cli/             CLI subcommands — `ttymap snap` (headless ANSI renderer)
│
├── app/                 (loop driver — sits between core and front)
│   ├── mod.rs           App::new / run / handle_event / handle_input / render_into
│   ├── event.rs         AppEvent — unified queue payload (Command / FrameReady / Input / Wake)
│   ├── frame_timer.rs   per-iteration wake source
│   └── ui.rs            ratatui draw entry — routes through compositor::render::paint
│
├── lua/                 Lua bridge (mlua + Lua 5.4 vendored). Spans core + front.
│   ├── mod.rs           plugin discovery + register_one + package.searchers wiring
│   ├── registry.rs      LuaEventBus — pub/sub for "tick" / "frame_ready" / "map_jumped" / …
│   ├── runtimepath.rs   Neovim-style runtime layer resolution
│   ├── init_lua.rs      separate config-DSL Lua state (ttymap.opt + ttymap.keymap)
│   ├── handle.rs        shared host handle plumbing
│   ├── sender.rs        channel sender helpers
│   ├── registrar.rs     activation collection bucket (Activation / PaletteEntry)
│   ├── api/             Rust→Lua binding (the `ttymap` global)
│   │   ├── mod.rs       install() orchestrator
│   │   ├── host_*.rs    one file per Lua-side namespace userdata (map / tile / config / help / log)
│   │   ├── register.rs  register_palette_command / register_keybind / on_event capturers
│   │   ├── imperative.rs ttymap.api.{card,palette,frame,notify} — runtime imperative primitives
│   │   ├── http.rs / json.rs / sgp4.rs / map_api.rs
│   └── bridge/          Lua spec → Rust trait adapters
│       ├── handle.rs / card_component.rs / card_handle.rs / card_parse.rs
│       └── palette_provider.rs / palette_handle.rs
│
└── shared/              foundation utilities (used by host, Lua bridge, tile fetcher)
    ├── geoip.rs         IP-based lat/lon lookup
    └── http/            user-agent-tagged reqwest wrapper

runtime/
├── init.lua             bundled defaults (Berlin auto-zoom, dark theme); user init.lua runs after this
├── plugin/              bundled plugins — auto-discovered, identity = file stem
│   │                    10 single-file plugins: attribution, scalebar, info, help, center, here,
│   │                    export, notify, ping_simulation, quake
│   └── aircraft/, satellite/, search/, wiki/   directory plugins (<plugin>/init.lua + sibling files)
└── lua/ttymap/          shared lib scripts (fmt, sidebar). Resolved via `require "ttymap.X"`
                         through a custom package.searchers entry that walks every runtime layer.
```

## Layering

The dependency graph runs **foundation → core → app → front**, with
`lua/` as a bridge that legitimately spans both. Concretely:

- **Foundation** types are imported by everyone but depend on
  nothing internal. `command.rs` (UserCommand vocabulary), `geo.rs`,
  `config.rs`, `theme/{palette,mod}.rs` (data only).
- **`core/`** imports foundation. Imports no `front/`, no `app/`.
  Verified ratatui-free at the import-graph level except for
  `core/compositor/{render,sidebar,window,map_api}.rs` which use
  ratatui internally for paint primitives — but the trait surface
  (`Component`) and state machinery (`Compositor`, `Op`,
  `BaseLayer`) doesn't.
- **`front/`** imports foundation + core. Houses the ratatui
  `UiTheme` adapter, the palette picker UI, and the CLI subcommand
  bucket.
- **`app/`** imports foundation + core + front. The loop driver
  above core: drains the bus, forwards commands to
  `core::Dispatcher`, calls `terminal.draw(...)`. Owns the rendered
  `MapFrame` snapshot.
- **`lua/`** is the bridge — Lua plugins implement core's
  `Component` trait but draw via front's `MapApi`, so this layer
  legitimately spans core + front.

Per-module roles:

- **`core/map/`** — domain. Knows nothing about UI, plugins, or
  focus. `MapAction` carries every map-level mutation, including
  mouse-continuous variants (`PanCells`, `ZoomAt`).
- **`core/dispatcher.rs`** — GoF Receiver. `UserCommand` (in
  `src/command.rs` at crate root) is the closed enum every input
  source emits; `Dispatcher::dispatch` is the sole place that
  executes them. Map-level actions nest under
  `UserCommand::Map(MapAction)`; other variants sit at the top
  level. The `UserCommand` type lives at the crate root rather than
  under `core/` so producers reach it via `crate::UserCommand`
  without an upward dependency on the Receiver. See
  [design.md](design.md) for the judgment rules.
- **`core/compositor/`** — focus and modal state. A stack of
  `Component`s; the top owns key focus. No `is_visible` /
  `activate` / `deactivate` contract — presence on the stack *is*
  the lifecycle. `Tab` / `Shift-Tab` cycle focus (framework-
  reserved). `Placement::Sidebar` cards stack equal-split in the
  left rail (max 3 visible); `Placement::Floating` draws over the
  map (palette).
- **`core/input/mouse.rs`** — pure adapter.
  `MouseEvent → Vec<UserCommand>` (`CursorMoved` on every event;
  drag → `Map(PanCells)`; scroll → `Map(ZoomAt)`). No state mutation.
- **`app/`** — loop driver. `App::run` drains the unified
  `AppEvent` bus, routes events to `core::Dispatcher`, and asks
  ratatui to paint each iteration. `App::handle_input` translates
  raw terminal events into `UserCommand`s pushed back through the
  bus. App stays at the top level (not under `front/`) for now;
  it's the orchestrator that owns the dispatcher and renders.
- **`app/ui.rs`** — non-modal shell. `draw()` paints the latest
  `MapFrame`, runs `LuaEventBus::dispatch_tick` so every Lua
  plugin's `on_tick` callback gets one frame to paint world-space
  markers via `MapApi`, then calls
  `core::compositor::render::paint(...)` for the focus stack.
  **The main thread doing per-frame Lua draw work is the one
  legitimate exception to "main thread only displays" — see
  [design.md](design.md).**
- **`front/palette/`** — `:`-triggered universal picker. Itself a
  `Component`; provider sub-modes (theme picker, search, plugin
  commands) swap in place via `PaletteAction::SwitchProvider`. Sync
  providers filter on each keystroke; async providers debounce and
  poll for results.
- **`lua/`** — every in-tree plugin. Plugins are auto-discovered
  `.lua` files under any runtime layer's `plugin/` dir; identity is
  the file stem. A script joins host loops by calling
  `ttymap.api.frame.on_tick(fn)` (per-frame work — sugar for
  `ttymap.on_event("tick", fn)`), `ttymap.on_event(name, fn)`
  (any host event: `frame_ready`, `map_jumped`, `theme_changed`,
  `resized`, …), `ttymap.register_palette_command({label, invoke})`
  (palette row), or `ttymap.register_keybind(key, fn)` (top-level
  keybind). Panels and palettes are opened *imperatively* from
  inside callbacks via `ttymap.api.card.open(spec)` /
  `ttymap.api.palette.open(spec)`. Drawing primitives (`MapApi`)
  live under `core/compositor/`; the Lua bridge wraps them via
  `Lua::scope` so the Lua-side handle never outlives a single tick.
  See [lua-architecture.md](lua-architecture.md) for the full
  surface.
- **`theme/` (foundation)** — colour palette data only
  (`ColorPalette`, `DARK` / `BRIGHT`, `ThemeId`). No ratatui
  dependency. The map renderer's styler reads `ColorPalette`
  directly; the UI chrome reads it via `front::theme::UiTheme`.
- **`front/theme/`** — ratatui adapter (`UiTheme`) + semantic-tag
  enum (`StyleKind`). Lua scripts ask for a tag string (`"accent"`
  / `"accent_alt"` / `"muted"` / …) and the bridge resolves it
  through the active `UiTheme` to a concrete colour. Lua plugins
  never see `UiTheme` directly.

## Message flow

```
raw event
  ↓ keyboard / mouse / Lua callback / tile arrival / frame timer
  ↓ produces 0..N UserCommand or AppEvent::FrameReady (pure translation)
  ↓
App::dispatch(intent)
  ↓
    UserCommand::Map(action)        → MapState::process_action(&action)
                                     (Map(MapAction::Jump(loc)) recentres,
                                      Map(MapAction::Pan…) scrolls, etc.)
    UserCommand::Quit               → break the event loop
    UserCommand::SetTheme(id)       → App::switch_theme (rebuilds styler + UI theme)
    UserCommand::CursorMoved(c,r)   → cursor overlay
    UserCommand::CycleFocus(fwd)    → Compositor::cycle
    UserCommand::Resize(cols,rows)  → App::handle_resize
    UserCommand::ToggleSidebar      → show/hide the sidebar; recomputes
                                     the map canvas so the render thread
                                     allocates the right buffer size
    UserCommand::ExportFrame        → write the current MapFrame as ANSI / HTML
```

Keyboard and mouse take different paths to `UserCommand` — keys go
through the Compositor; mouse events go through a pure adapter:

```
key event
  ↓ Compositor::handle_event(event, ctx):
    [reserved]  Tab / Shift-Tab   → UserCommand::CycleFocus(…)
    [focused]   focused component's handle_event(event, &mut win)
                  ↓ win.emit / win.open / win.close / win.ignore
    [fallback]  only if the focused component called win.ignore()
                and focus isn't already on BaseLayer
                → re-deliver to BaseLayer (keymap + activation table + count prefix)
  ↓ Vec<UserCommand>

mouse event
  ↓ MouseAdapter::translate(event) → Vec<UserCommand>:
    every event   → UserCommand::CursorMoved(col, row)
    drag (left)   → UserCommand::Map(MapAction::PanCells(dx, dy))
    scroll        → UserCommand::Map(MapAction::ZoomAt { anchor_*, zoom_in })
```

## Render flow

Rendering is decoupled from fetching. The render thread builds a
`MapFrame` from the current `Viewport`; the main thread consumes it.
Stale frames are fine — overlays reproject against the frame's own
center/zoom (`MapFrame` carries the view it was rendered at).

```
main thread (ratatui draw):
  ui::draw(f, &compositor, &theme, &ctx):
    1. latest MapFrame is painted into the map area
    2. LuaEventBus::dispatch_tick — every Lua plugin's on_tick callback
       runs once with a scoped MapApi handle, drawing world-space
       primitives (aircraft / satellite / quake / wiki markers, scale
       bar, attribution, …) and queueing UserPolyline overlays
    3. compositor renders the focused stack surfaces:
       - the topmost Floating component (palette is the only one) is
         drawn over the map area
       - Sidebar cards share the left rail via equal vertical split
         (oldest at top, max 3 visible) when the sidebar is open
    4. footer hints from the focused component
    5. drained polyline sink → next RenderTask::Draw{viewport, overlays}
```

## Focus model

Focus is a `focused_idx` into the Compositor stack, **decoupled from
stack position**. Pushing a modal puts focus on it; `Tab` moves
focus back to the base layer without popping the modal (the old
`Focus::Background` behaviour). Stack order never changes through
cycling — only which component receives keys first.

**No framework-side dedup.** nvim-style: pressing an activation key
twice produces two instances of the plugin on the stack. Plugins
that want toggle behaviour close themselves in their own
`handle_event` (typically: capture the `CardHandle` returned by
`api.card.open`, and on the activation key call `:close()` and nil
out the handle). This keeps the Rust core ignorant of plugin
identity — the concrete-type / `dedup_tag` schemes the compositor
used to enforce all came at the cost of plugins still having to
re-implement close-and-toggle on top, so we removed them.

## Concurrency

| Thread | Responsibility |
|--------|----------------|
| main | event loop, compositor, Lua dispatch, UI state, terminal draw |
| render | MapFrame generation (tile fetch + draw) |
| tile fetch | HTTP workers with priority queue |
| Lua `ttymap.http:fetch` | one short-lived OS thread per request (Nominatim / Wikipedia / geoip / ADS-B / TLE / USGS) — Lua side polls `job:try_take()` |

crossbeam channels connect the threads; the main thread never blocks
on I/O. The render thread parks on a `crossbeam::select!` over its
task channel and a wake channel pinged by the decoder thread on each
tile arrival — no timeout-based polling.
