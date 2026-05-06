# Architecture

This is the system-level reference: module layout, threads, message flow,
render flow, focus model. For plugin authoring see
[lua-architecture.md](lua-architecture.md). For load-bearing design
decisions and trade-offs see [design.md](design.md).

## Source tree & layering

The canonical source-tree layout and layering rule live in
[../CLAUDE.md → Workspace layout / Source tree](../CLAUDE.md). In short:

- The workspace has two crates: `ttymap-engine/` (headless render
  pipeline, ratatui- and crossterm-free) and `ttymap-tui/` (TUI
  binary; produces the `ttymap` executable).
- The single layering rule is enforced at the **crate** level: the
  engine doesn't depend on ratatui or crossterm. Inside the binary,
  modules are flat peers (`app/`, `cli/`, `compositor/`, `input/`,
  `palette/`, `theme/`, `lua/`, `shared/`).
- The earlier `core/` / `front/` split (issue #212 Phase 4) was
  reverted to a flat layout — see CLAUDE.md for the rationale.

Per-module roles are described once, in [../CLAUDE.md → Key
modules](../CLAUDE.md). This document focuses on cross-module
mechanics (message flow, render flow, focus, threads).

## Message flow

```
raw event
  ↓ keyboard / mouse / Lua callback / tile arrival / frame timer
  ↓ produces 0..N UserCommand or AppEvent (FrameReady / Input / Wake / Bus)
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
```

Keyboard and mouse take different paths to `UserCommand` — keys go
through the Compositor; mouse events go through a pure adapter:

```
key event
  ↓ Compositor::handle_key(event, ctx):
    [reserved]  Tab / Shift-Tab   → UserCommand::CycleFocus(…)
    [focused]   focused component's handle_key(event, &mut win)
                  ↓ win.emit / win.open / win.close / win.ignore
    [fallback]  only if the focused component called win.ignore()
                and focus isn't already on BaseLayer
                → re-deliver to BaseLayer (keymap + activation table + `gg` sequence)
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
    2. lua::tick::dispatch_tick — every Lua plugin's on_tick callback
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
`handle_key` (typically: capture the `CardHandle` returned by
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
