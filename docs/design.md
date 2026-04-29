# Design philosophy

This document captures the "why" behind a few load-bearing decisions in
the codebase, and the judgment criteria we use when facing similar
decisions again.

## Command pipeline vs direct API

### The rule

**User / system intent goes through `App::dispatch`. Internal data
flow does not.**

Anything that's "we want to do X in response to an event" becomes an
`AppMsg` variant and flows through `App::dispatch(msg)` on the single
`App` receiver. Anything that's "a worker finished its job and handed
us the result" stays as a direct method call.

### Why a pipeline at all

`App::dispatch` is the **single side-effect boundary** for app-level
state changes. `App` is the Receiver (GoF): every invoker (keymap,
palette, plugins, mouse) returns `Vec<AppMsg>` and never executes
anything itself. Each match arm either delegates to a method on the
domain type that owns the relevant state (`MapState`) or — for
cross-cutting transitions — to a method on `App` itself
(`apply_theme`, `handle_resize`). Arms whose effect changed the map
frame call `self.request_map_redraw()` inline to request a fresh
frame and notify passive widgets. This gives us:

- One place to audit what can happen to app state
- One place where the redraw-after-map-change invariant lives
- A shared vocabulary for keymap, palette providers, plugin async
  callbacks, mouse — they all emit the same `AppMsg` enum

Without the pipeline, that redraw rule would need to be duplicated at
every call site that mutates the map.

Note on naming: "command" is reserved for user-facing concepts — the
CLI subcommand under `src/commands/` and the palette entries under
`src/plugin/palette/`. The internal intent type is `AppMsg` so those
three layers stay unambiguous.

### When to emit an `AppMsg`

Emit an `AppMsg` if **all** of the following are true:

1. It represents an **intent** (user action, OS event, plugin wanting
   something to happen). It's not a completion notification.
2. The effect is meaningful at app level — it changes what the app is
   doing, not just a worker's internal state.
3. Frequency is low enough that the dispatch overhead doesn't matter.

Current examples:

| AppMsg              | Source                          | Why it is an AppMsg                    |
| ------------------- | ------------------------------- | -------------------------------------- |
| `Map(Action::Pan…)` | keymap, mouse drag              | User intent → map state change         |
| `Map(Action::Quit)` | keymap `q`, palette `:q`, Ctrl-C | Same intent from 3 sources             |
| `Map(Action::Redraw)` | initial draw                  | Forces an unconditional fresh frame    |
| `Resize(w, h)`      | crossterm `Resize` event        | Cross-cutting: map state + render canvas |
| `SetTheme`          | palette entry                   | Cross-cutting: UI theme + render styler |
| `CursorMoved`       | mouse router (every event)      | Overlay readout through the same boundary |
| `Jump(LonLat)`      | search provider result, geoip plugin | Plugin async → map state change   |
| `CycleFocus`        | Tab / Shift-Tab                 | UI transition                          |

Surface activations (palette open, plugin activate) deliberately do
*not* go through `AppMsg` — they're expressed as a `Component` push
onto the compositor stack, queued through `Window::open` /
`Window::toggle` from the `BaseLayer`'s activation table and applied
atomically after the hook returns. Routing focus through `AppMsg`
would force the dispatch table to know which surfaces exist; keeping
it on the compositor side means new plugins add zero `AppMsg`
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
| `Task::poll() -> Vec<AppMsg>`          | Every tick, headless plugins (here, …)      |
| `render_handle.request_draw(…)`        | Sending to another thread (not app state)   |

### The infinite-loop trap

Naively wrapping "frame arrived" as `AppMsg::FrameArrived(frame)` is
tempting — everything goes through the same pipeline, right? It breaks:

```
frame arrives → AppMsg::FrameArrived
             → dispatch → map_frame = Some(f)
             → request_map_redraw → render thread renders a frame
             → frame arrives → AppMsg::FrameArrived
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

- **YES** → probably an `AppMsg`. The pipeline ensures the related
  changes fire consistently.
- **NO** → direct method. An `AppMsg` just adds ceremony without
  earning anything.

## Controller split: by feature, not by domain

When `App::dispatch` grows past ~200 lines (together with the
cross-cutting helpers it delegates to), split the router + helpers
into per-feature modules attached to `App` via `impl App { … }`
blocks (file names illustrative):

```
app/
  mod.rs          # App struct + top-level run/dispatch
  msg.rs          # AppMsg enum definition
  map_msg.rs      # impl App for Map / Jump
  theme.rs        # impl App for apply_theme
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

Modal surfaces (palette, wiki, help, …) are `Component`s on
a stack owned by the `Compositor`. Pushed on activation, popped when
the component calls `win.close()`. Focus is a separate `focused_idx`
into the stack, so `Tab` can move focus to the base layer without
popping the modal above it.

### Why a stack instead of `is_visible` / `activate` / `deactivate`

A flag-driven design forces every surface to keep two pieces of
state in sync — "am I on screen?" and "do I own focus?" — and every
plugin author has to re-implement the activation lifecycle. Drift
between them is the bug class the old `FocusManager` + `Plugin`
trio kept generating.

Stack presence collapses both into one fact: the component object
**exists** while it is visible, is **dropped** the instant it
closes. There is no second flag to forget.

This lets `Component::paint_on_map` (world-space markers like the
wiki article pins) be unconditional — every component on the stack
paints. Markers naturally appear when the panel opens and disappear
when it closes, with no "is this paint still active?" check.

### Why a `&mut Window` queue instead of returning a result enum

Hooks (`handle_event`, `poll`) receive a `&mut Window`. Plugins
record intent through it — `win.close()`, `win.open(c)`,
`win.emit(msg)`, `win.ignore()` — and the compositor drains the
queue after the hook returns, applying ops atomically in a fixed
order: `close` → `opens` (TypeId dedup) → `toggles` → `msgs`.

A return-value alternative (`EventResult::CloseAndPush(...)`) does
not scale: every new compound op needs a new variant. A queue
expresses compounds by composition.

The handle never grants `&mut Compositor`. Plugins cannot mutate
the stack directly; the compositor is the sole applier. Stack /
focus / dedup invariants stay framework-enforced regardless of
plugin bugs.

### Type-id dedup

`push_or_focus` uses `Any::type_id` to detect that a component of
the same concrete type is already on the stack — focus jumps to the
existing instance instead of stacking a duplicate. A plugin author
cannot forget to opt in: the concrete type *is* the dedup identity.

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

`MapFrame` (`src/map/render/frame.rs`) is a finished grid of
`MapCell { ch, fg, bg }`. All tile fetching, spatial indexing,
styling, polygon fill, line drawing, label placement, Braille
packing, and color assignment happen on the render thread before a
`MapFrame` is produced.

The main thread's job is to **display** them. It should not peek
inside `cells` or recompute anything. This lets the render thread
stay CPU-heavy without blocking input, and lets the main thread drop
older frames when multiple are queued (latest wins).
