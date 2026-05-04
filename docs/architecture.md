# Architecture

This is the system-level reference: module layout, threads, message flow,
render flow, focus model. For plugin authoring see
[lua-architecture.md](lua-architecture.md). For load-bearing design
decisions and trade-offs see [design.md](design.md).

## Source tree

```
src/
├── main.rs              CLI entry + interactive / snap dispatch
├── lib.rs               crate root
├── logging.rs           XDG state log (--log flag)
├── config.rs            Config struct + Default impls (loaded from init.lua)
├── geo.rs               Web Mercator, lon/lat ↔ tile coords, distance
│
├── theme/               colour palette + ratatui adapter + StyleKind tags
│   ├── mod.rs           ThemeId + re-exports
│   ├── palette.rs       ColorPalette + DARK / BRIGHT consts (xterm-256 indices)
│   ├── style.rs         StyleKind semantic tags (accent / muted / …)
│   └── ui.rs            UiTheme (ratatui style adapter)
│
├── input/               input subsystem — peer of map/, app/, lua/
│   ├── thread.rs        producer thread blocking on crossterm::event::read
│   ├── keymap.rs        KeyMap table + KeybindingOverrides (init.lua + Lua keymap.set)
│   └── mouse.rs         MouseAdapter — MouseEvent → UserIntent (drag pan, scroll zoom-at)
│
├── app/                 controller: event loop, intent dispatch, UI shell
│   ├── mod.rs           App::new / run / dispatch — single side-effect boundary
│   ├── intent.rs        UserIntent enum (Map(MapAction) / Quit / SetTheme / ToggleSidebar / …)
│   ├── event.rs         AppEvent — unified queue payload (Intent / FrameReady / Input / Wake)
│   ├── frame_timer.rs   per-iteration wake source
│   └── ui.rs            ratatui draw entry — map area + footer, routes through compositor
│
├── compositor/          helix-style focus / modal stack (top-level peer of app/)
│   ├── mod.rs           Compositor + Component trait + Placement (Floating / Sidebar) + Registrar
│   ├── base.rs          BaseLayer — keymap + activation table + count prefixes (5j etc.)
│   ├── sidebar.rs       left-rail layout (up to 3 Sidebar cards, equal vertical split)
│   ├── window.rs        Window (event-side, queues ops) + RenderWindow (render-side, owns UiTheme)
│   ├── op.rs            Op enum — single drain vocabulary (Push / Close / Intent)
│   └── map_api.rs       MapApi — world-space + screen-space draw primitives consumed by Lua tick
│
├── palette/             `:`-triggered universal picker (itself a Component)
│   ├── mod.rs           CommandPalette ephemeral state
│   ├── action.rs        PaletteAction (close / submit / SwitchProvider / …)
│   ├── panel.rs         popup layout
│   └── provider/        sync (OnEachKey filter) + async (Debounced + poll + spinner) plumbing
│
├── commands/            CLI subcommands (one file per subcommand)
│   ├── mod.rs
│   └── snap.rs          `ttymap snap` — headless single-frame ANSI renderer
│
├── lua/                 Lua bridge (mlua + Lua 5.4 vendored). All in-tree plugins.
│   ├── mod.rs           plugin discovery + register_one + package.searchers wiring
│   ├── registry.rs      LuaEventBus — pub/sub for "tick" / "frame_ready" / "map_jumped" / …
│   ├── runtimepath.rs   Neovim-style runtime layer resolution (env / dev / xdg_config / xdg_data)
│   ├── init_lua.rs      separate config-DSL Lua state (ttymap.opt + ttymap.keymap)
│   ├── handle.rs        shared host handle plumbing
│   ├── sender.rs        channel sender helpers (push to App via crossbeam)
│   ├── api/             Rust→Lua API binding (the `ttymap` global)
│   │   ├── mod.rs       install() + namespace userdata (register_palette_command, register_keybind, …)
│   │   ├── http.rs      ttymap.http:fetch — background GET returning a poll-able Job
│   │   ├── json.rs      ttymap.json:parse / encode
│   │   ├── map_api.rs   per-frame MapApi → Lua table (Lua::scope, no leak across ticks)
│   │   └── sgp4.rs      ttymap.sgp4 — TLE → lon/lat propagation (used by satellite plugin)
│   └── bridge/          Lua → Rust trait adapters
│       ├── handle.rs    LuaHandle dispatch plumbing
│       ├── card_component.rs  LuaCardComponent (Component impl for ttymap.api.card.open spec)
│       ├── card_handle.rs     CardHandle + CloseFlag (returned to Lua)
│       ├── card_parse.rs      Lua-table → CardSpec parsing
│       ├── palette_provider.rs LuaPaletteProvider (PaletteProvider impl)
│       └── palette_handle.rs   PaletteHandle (mirror of CardHandle)
│
├── shared/              host-and-Lua-bridge utilities
│   ├── geoip.rs         IP-based lat/lon lookup (used by --here flag and `here` plugin)
│   └── http/            user-agent-tagged reqwest wrapper
│
└── map/                 domain — viewport state + rendering pipeline
    ├── mod.rs, state.rs, action.rs
    ├── render/          tiles → MapFrame on a dedicated thread
    │   ├── pipeline.rs, thread.rs, renderer.rs
    │   ├── canvas.rs, braille.rs, frame.rs, frame_widget.rs, view.rs
    │   ├── label.rs, overlay.rs, polygon.rs, project.rs, geom/
    │   └── earcut_worker.rs, panic_silence.rs
    ├── styler/          Mapbox GL-style rules — schema/mapscii.rs single source; theme swaps ColorPalette only
    └── tile/            MVT fetch + cache + decode
        ├── cache.rs         Memory LRU + view state + prefetch + DiskFastPath (render-thread sync read)
        ├── decoder.rs       Relay thread: bytes → DecodedTile (off the render thread)
        ├── disk.rs          On-disk tile read/write helpers (shared by fast path + decorator)
        ├── key.rs, property.rs
        ├── decode/          Protobuf → DecodedTile (geometry / tags / decompress)
        └── fetch/           TileFetcher trait + FetchLane (lane / queue / priority) + http + disk_cached decorator

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

- **`map/`** — domain. Knows nothing about UI, plugins, or focus.
  `MapAction` carries every map-level mutation, including mouse-continuous
  variants (`PanCells`, `ZoomAt`).
- **`app/`** — the **controller**. `UserIntent` (in
  `app/intent.rs`) is the closed enum every input source (keymap,
  palette, compositor components, mouse adapter, Lua callbacks, async
  tasks) emits; `App::dispatch` in `app/mod.rs` is the sole
  place that executes them. Map-level actions nest under
  `UserIntent::Map(MapAction)`; other variants sit at the top level.
  Command pattern with `App` as the Receiver — see
  [design.md](design.md) for the UserIntent-vs-direct-call judgment
  rules.
- **`compositor/`** — focus and modal state. A stack of
  `Component`s; the top owns key focus. No `is_visible` / `activate`
  / `deactivate` contract — presence on the stack *is* the lifecycle.
  `Tab` / `Shift-Tab` cycle focus (framework-reserved, intercepted
  before any component sees them). `Placement::Sidebar` cards stack
  equal-split in the left rail (max 3 visible);
  `Placement::Floating` draws over the map (palette).
- **`input/mouse.rs`** — pure adapter.
  `MouseEvent → Vec<UserIntent>` (`CursorMoved` on every event;
  drag → `Map(PanCells)`; scroll → `Map(ZoomAt)`). No state mutation.
- **`app/ui.rs`** — non-modal shell. `draw()` paints the latest
  `MapFrame`, runs the `LuaEventBus::dispatch_tick` so every Lua
  plugin's `on_tick` callback gets one frame to paint world-space
  markers via `MapApi`, then forwards modal rendering to the
  Compositor. **The main thread doing per-frame Lua draw work is the
  one legitimate exception to "main thread only displays" —
  see [design.md](design.md).**
- **`palette/`** — `:`-triggered universal picker. Itself a
  `Component`; provider sub-modes (theme picker, search, plugin
  commands) swap in place via `PaletteAction::SwitchProvider`. Sync
  providers filter on each keystroke; async providers debounce and
  poll for results.
- **`lua/`** — every in-tree plugin. Plugins are auto-discovered
  `.lua` files under any runtime layer's `plugin/` dir; identity is
  the file stem. A script joins host loops by calling
  `ttymap.api.frame.on_tick(fn)` (per-frame work — sugar for
  `ttymap.on_event("tick", fn)`),
  `ttymap.on_event(name, fn)` (any host event:
  `frame_ready`, `map_jumped`, `theme_changed`, `resized`, …),
  `ttymap.register_palette_command({label, invoke})` (palette row),
  or `ttymap.register_keybind(key, fn)` (top-level keybind). Panels
  and palettes are opened *imperatively* from inside callbacks via
  `ttymap.api.card.open(spec)` / `ttymap.api.palette.open(spec)`.
  Drawing primitives (`MapApi`) live under `compositor/`;
  the Lua bridge wraps them via `Lua::scope` so the Lua-side handle
  never outlives a single tick. See
  [lua-architecture.md](lua-architecture.md) for the full surface.
- **`theme/`** — palette data + `UiTheme` ratatui adapter +
  `StyleKind` semantic tags. Lua scripts ask for a tag string
  ("accent" / "accent_alt" / "muted" / …) and the bridge resolves
  it through the active `UiTheme` to a concrete colour. Lua plugins
  never see `UiTheme` directly.

## Message flow

```
raw event
  ↓ keyboard / mouse / Lua callback / tile arrival / frame timer
  ↓ produces 0..N UserIntent or AppEvent::FrameReady (pure translation)
  ↓
App::dispatch(intent)
  ↓
    UserIntent::Map(action)        → MapState::process_action(&action)
                                     (Map(MapAction::Jump(loc)) recentres,
                                      Map(MapAction::Pan…) scrolls, etc.)
    UserIntent::Quit               → break the event loop
    UserIntent::SetTheme(id)       → App::switch_theme (rebuilds styler + UI theme)
    UserIntent::CursorMoved(c,r)   → cursor overlay
    UserIntent::CycleFocus(fwd)    → Compositor::cycle
    UserIntent::Resize(cols,rows)  → App::handle_resize
    UserIntent::ToggleSidebar      → show/hide the sidebar; recomputes
                                     the map canvas so the render thread
                                     allocates the right buffer size
    UserIntent::ExportFrame        → write the current MapFrame as ANSI / HTML
```

Keyboard and mouse take different paths to `UserIntent` — keys go
through the Compositor; mouse events go through a pure adapter:

```
key event
  ↓ Compositor::handle_event(event, ctx):
    [reserved]  Tab / Shift-Tab   → UserIntent::CycleFocus(…)
    [focused]   focused component's handle_event(event, &mut win)
                  ↓ win.emit / win.open / win.close / win.ignore
    [fallback]  only if the focused component called win.ignore()
                and focus isn't already on BaseLayer
                → re-deliver to BaseLayer (keymap + activation table + count prefix)
  ↓ Vec<UserIntent>

mouse event
  ↓ MouseAdapter::translate(event) → Vec<UserIntent>:
    every event   → UserIntent::CursorMoved(col, row)
    drag (left)   → UserIntent::Map(MapAction::PanCells(dx, dy))
    scroll        → UserIntent::Map(MapAction::ZoomAt { anchor_*, zoom_in })
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
