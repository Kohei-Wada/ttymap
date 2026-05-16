# Design philosophy

This document captures the "why" behind a few load-bearing decisions in
the codebase, and the judgment criteria we use when facing similar
decisions again.

## Command pipeline vs direct API

### The rule

**User / system intent goes through `App::dispatch`. Internal data
flow does not.**

Anything that's "we want to do X in response to an event" becomes an
`UserCommand` variant and flows through `App::dispatch(msg)` on the single
`App` receiver. Anything that's "a worker finished its job and handed
us the result" stays as a direct method call.

### Why a pipeline at all

`App::dispatch` is the **single side-effect boundary** for app-level
state changes. `App` is the Receiver (GoF): every invoker (keymap,
palette, plugins, mouse) returns `Vec<UserCommand>` and never executes
anything itself. Each match arm either delegates to a method on the
domain type that owns the relevant state (`MapState`) or — for
cross-cutting transitions — to a method on `App` itself
(`switch_theme`, `handle_resize`). Arms whose effect changed the map
frame call `self.request_map_redraw()` inline to request a fresh
frame and notify passive widgets. This gives us:

- One place to audit what can happen to app state
- One place where the redraw-after-map-change invariant lives
- A shared vocabulary for keymap, palette providers, plugin async
  callbacks, mouse — they all emit the same `UserCommand` enum

Without the pipeline, that redraw rule would need to be duplicated at
every call site that mutates the map.

Note on naming: "command" is reserved for user-facing concepts — the
CLI subcommand under `ttymap-cli/src/` and the palette entries under
`ttymap-tui/src/palette/`. The internal intent type is `UserCommand` so those three
layers stay unambiguous.

### When to emit a `UserCommand`

Emit a `UserCommand` if **all** of the following are true:

1. It represents an **intent** (user action, OS event, plugin wanting
   something to happen). It's not a completion notification.
2. The effect is meaningful at app level — it changes what the app is
   doing, not just a worker's internal state.
3. Frequency is low enough that the dispatch overhead doesn't matter.

Current examples:

| UserCommand                          | Source                                | Why it is a UserCommand                    |
| ----------------------------------- | ------------------------------------- | ----------------------------------------- |
| `Map(MapAction::Pan…)`              | keymap, mouse drag                    | User intent → map state change            |
| `Map(MapAction::Jump(LonLat))`      | search provider result, geoip plugin  | Plugin async → map state change           |
| `Quit`                              | keymap `q`, palette `:q`, Ctrl-C      | Same intent from 3 sources                |
| `Resize(w, h)`                      | crossterm `Resize` event              | Cross-cutting: map state + render canvas  |
| `SetTheme`                          | palette entry                         | Cross-cutting: UI theme + render styler   |
| `CursorMoved`                       | mouse router (every event)            | Overlay readout through the same boundary |
| `CycleFocus`                        | Tab / Shift-Tab                       | UI transition                             |
| `ToggleSidebar`                     | keymap, palette                       | Cross-cutting: visibility + map canvas    |

Surface activations (palette open, plugin activate) deliberately do
*not* go through `UserCommand` — they're expressed as a `Component` push
onto the compositor stack, queued through `Window::open` from inside
a `Component`'s `handle_key` (or directly from
`api.card.open` / `api.palette.open` on the Lua side) and applied
atomically after the hook returns. Routing focus through `UserCommand`
would force the dispatch table to know which surfaces exist; keeping
it on the compositor side means new plugins add zero `UserCommand`
variants.

### When to use a direct method / setter instead

Use a plain method call when **any** of the following is true:

1. It's a **completion notification** from a worker (render thread
   finished a frame; tile fetch finished downloading).
2. It's **periodic maintenance** (widget `poll()` tick, throttle timer).
3. It's **high-frequency** (many per second) and dispatch overhead
   would dominate.
4. Routing through the dispatcher would be **wrong** (infinite loop
   with completion notifications).

Current examples:

| Operation                              | Where                                       |
| -------------------------------------- | ------------------------------------------- |
| `render_handle.try_recv_frame()` loop  | Every tick, pulls completed MapFrames       |
| `Component::poll(&mut win)`            | Every tick, every component on the stack    |
| `Task::poll() -> Vec<UserCommand>`          | Every tick, headless plugins (here, …)      |
| `render_handle.request_draw(…)`        | Sending to another thread (not app state)   |

### The infinite-loop trap

Naively wrapping "frame arrived" as `UserCommand::FrameArrived(frame)` is
tempting — everything goes through the same pipeline, right? It breaks:

```
frame arrives → UserCommand::FrameArrived
             → dispatch → map_frame = Some(f)
             → request_map_redraw → render thread renders a frame
             → frame arrives → UserCommand::FrameArrived
             → dispatch → map_frame = Some(f)
             → request_map_redraw → …
```

To avoid this you'd have to carve out a special no-redraw arm, which
defeats the uniformity the pipeline buys you. A direct
`ui.drain_frames(…)` call sidesteps the loop entirely and is the right
shape.

### The decision question

When unsure, ask:

> "If this happens, should **other state also change** in response?"

- **YES** → probably a `UserCommand`. The pipeline ensures the related
  changes fire consistently.
- **NO** → direct method. An `UserCommand` just adds ceremony without
  earning anything.

## Controller split: by feature, not by domain

When `App::dispatch` grows past ~200 lines (together with the
cross-cutting helpers it delegates to), split the router + helpers
into per-feature modules attached to `App` via `impl App { … }`
blocks (file names illustrative):

```
app/
  mod.rs          # App struct + top-level run/dispatch
  msg.rs          # UserCommand enum definition
  map_msg.rs      # impl App for Map / Jump
  theme.rs        # impl App for switch_theme
  resize.rs       # impl App for handle_resize
```

Do **not** split by state domain (`map.rs` / `ui.rs` / `render.rs`).
Many messages are cross-cutting (e.g. `SetTheme` touches both UI theme
and render styler; `Resize` touches map state and render canvas). A
domain split forces these into arbitrary owners. A feature split lets
each module freely touch whatever state its feature needs, and `App`
stays the single Receiver.

Current threshold: not yet. Revisit when dispatch plus cross-cutting
methods push us over.

## Compositor: object lifetime is the visibility lifecycle

Focus surfaces (palette, wiki, help, …) are `Component`s on
a stack owned by the `Compositor`. Pushed on activation, popped when
the component calls `win.close()`. Focus is a separate `focused_idx`
into the stack, so `Tab` can move focus to the base layer without
popping the panel above it.

`Placement` has two variants: `Floating` (palette-only, drawn over
the map area) and `Sidebar` (left rail, vertical split, up to 3
visible cards). All Lua plugins land in `Sidebar`; Lua scripts have
no way to ask for `Floating`.

### Why a stack instead of `is_visible` / `activate` / `deactivate`

A flag-driven design forces every surface to keep two pieces of
state in sync — "am I on screen?" and "do I own focus?" — and every
plugin author has to re-implement the activation lifecycle. Drift
between them is the bug class the old `FocusManager` + `Plugin`
trio kept generating.

Stack presence collapses both into one fact: the component object
**exists** while it is visible, is **dropped** the instant it
closes. There is no second flag to forget.

Plugin map paint runs through `ttymap.api.frame.on_tick(fn)` (Lua),
which fires only while the plugin is "open" by the plugin's own
convention (typically: a captured `w` window-handle ref that's nil
when closed). Stack presence and map paint stay in step because the
Lua plugin gates its `on_tick` body on the same `w` it nils inside
its `close()`.

### Why a `&mut Window` queue instead of returning a result enum

Hooks (`handle_key`, `poll`) receive a `&mut Window`. Plugins
record intent through it — `win.close()`, `win.open(c)`,
`win.emit(msg)`, `win.ignore()` — and the compositor drains the
queue after the hook returns, applying ops atomically. The drain
vocabulary is a single `Op` enum with three variants (`Push`,
`Close`, `Intent`); ops are applied in arrival order and there is no
framework-side dedup or toggle stage.

A return-value alternative (`EventResult::CloseAndPush(...)`) does
not scale: every new compound op needs a new variant. A queue
expresses compounds by composition.

The handle never grants `&mut Compositor`. Plugins cannot mutate
the stack directly; the compositor is the sole applier. Stack and
focus invariants stay framework-enforced regardless of plugin bugs.

### Why no framework-side dedup

Earlier iterations enforced "one component per type" via
`Any::type_id` so a second activation focused the existing instance
instead of stacking a duplicate. We removed it: plugins that wanted
"open or focus existing" still had to write the close branch
themselves (toggle behavior for re-press of the activation key),
and plugins that legitimately wanted multiple instances (multi-card
panels) were fighting the dedup. The Rust core now stays ignorant of
plugin identity; toggle is a plugin-side policy decision (capture
the `CardHandle` returned by `open`, call `:close()` on the same
key). See [architecture.md](architecture.md) for the focus model.

## Cleanup via `Drop`, not manual

If a subsystem owns a thread or channel, make its handle implement
`Drop` and clean up there. Callers should not need to remember a
`shutdown()` / `close()` call.

Example: `RenderHandle` implements `Drop` which calls `self.shutdown()`.
`App` holds a `RenderHandle` as a field; when `main()` returns, `App`
drops, the handle drops, shutdown runs automatically.

The only reason to call a cleanup method explicitly is if you need it
to happen **before** the normal drop point (e.g. release a resource
before restoring the terminal). If no such ordering requirement exists,
let `Drop` do it.

## Frames are "completed products", not signals

`MapFrame` (`ttymap-engine/src/map/render/frame.rs`) is a finished grid of
`MapCell { ch, fg, bg }`. All tile fetching, spatial indexing,
styling, polygon fill, line drawing, label placement, Braille
packing, and color assignment happen on the render thread before a
`MapFrame` is produced.

The main thread's job is to **display** them. It should not peek
inside `cells` or recompute anything. This lets the render thread
stay CPU-heavy without blocking input, and lets the main thread drop
older frames when multiple are queued (latest wins).

The one legitimate exception is **per-frame Lua plugin work**:
`ui::draw` runs `lua::tick::dispatch_tick` so every plugin's
`on_tick` callback gets one frame to paint world-space primitives
via `MapApi`, and to push polylines into the overlay sink (drained
into the next `RenderTask::Draw`'s `overlays` field). This is
small per-plugin work — a few `MapApi::point` / `polyline` /
`label` calls — and pushes the heavy work (tile fetch, projection,
labelling, polygon fill, Braille packing) to the render thread
where it belongs. If a plugin's `on_tick` ever became the bottleneck,
the right move is to push *that* plugin's compute back to a worker
thread, not to redesign the boundary.

## Error boundary policy

The engine has one explicit error boundary: **anything reachable
from `ttymap-app` through `pub` items in `ttymap-engine` must
return `Result<_, EngineError>` instead of panicking**. Inside
that boundary, hot paths keep `unwrap` / `expect`.

The cut, in concrete terms:

- **Public boundary** (`Result<_, EngineError>`): `map::build`,
  `map::tile::build`, `shared::http::HttpClient::new` /
  `with_timeout`, and any future public spawn / construction
  helpers. The TUI binary is the sole external caller — it
  funnels these to `log::error!` / stderr instead of crashing.
  `EngineError` is defined in `ttymap-engine/src/error.rs`
  (thiserror) and folds the existing `shared::http::FetchError`
  in as a `#[from]` variant so narrow callers (e.g. the Lua
  `ttymap.http` bridge) can keep using the tighter type.
- **Internal hot paths** keep their `unwrap`s: the decoder's
  zigzag stream, the renderer's coordinate / clipping math,
  Braille bit packing, earcut triangulation, polyline antimeridian
  split. These are per-frame / per-tile invariants — surfacing
  them through `Result` would only add an error path on every
  draw with no real recovery for the caller.
- **`catch_unwind` islands** (`render::thread::run_loop`,
  `tile::decoder::spawn_decoder`) are intentional: a panic
  inside one render frame or one tile decode is contained,
  logged, and the worker keeps serving. These are *not* a
  replacement for the public-boundary `Result` rule — they
  exist because we cannot afford to tear down the whole
  pipeline for one bad tile.
- **Mutex poisoning** (`fetch/lane.rs`) currently panics on
  poison. This is out of scope here — issue #86 owns the
  mutex-poison policy.

When adding a new engine public API: if the operation can fail
in a way the caller might want to recover from (config / disk /
network / spawn), return `Result<_, EngineError>` and add a
variant if needed. If the failure is an internal invariant
violation, `unwrap`/`expect` is fine — but consider whether the
call site really belongs on the public boundary or should be
hidden behind a constructor that already absorbed the
invariant.
