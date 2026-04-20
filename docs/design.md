# Design philosophy

This document captures the "why" behind a few load-bearing decisions in
the codebase, and the judgment criteria we use when facing similar
decisions again.

## Command pipeline vs direct API

### The rule

**User / system intent goes through `command::dispatch`. Internal data
flow does not.**

Anything that's "we want to do X in response to an event" becomes a
`Command` variant and flows through `command::dispatch(cmd, &mut ctx)`.
Anything that's "a worker finished its job and handed us the result"
stays as a direct method call.

### Why a pipeline at all

`command::dispatch` is the **single side-effect boundary** for
app-level state changes. Every `Command` arm reads/writes exactly the
state it needs through the `DispatchCtx` bundle, and the post-dispatch
rule (`InputEffect::Map → request redraw`) lives in one place. This
gives us:

- One place to audit what can happen to app state
- One place where the redraw-after-map-change invariant lives
- A shared vocabulary for keymap, palette providers, plugin async
  callbacks, mouse — they all emit the same `Command` enum

Without the pipeline, that redraw rule would need to be duplicated at
every call site that mutates the map.

### When to emit a `Command`

Emit a `Command` if **all** of the following are true:

1. It represents an **intent** (user action, OS event, plugin wanting
   something to happen). It's not a completion notification.
2. The effect is meaningful at app level — it changes what the app is
   doing, not just a worker's internal state.
3. Triggering the post-dispatch redraw rule is correct (or harmless).
4. Frequency is low enough that the dispatch overhead doesn't matter.

Current examples:

| Command             | Source                          | Why a Command                          |
| ------------------- | ------------------------------- | -------------------------------------- |
| `Map(Action::Pan…)` | keymap, mouse drag              | User intent → map state change         |
| `Map(Action::Quit)` | keymap `q`, palette `:q`, Ctrl-C | Same intent from 3 sources             |
| `Map(Action::Redraw)` | initial draw                  | Forces post-dispatch redraw rule to fire |
| `Resize(w, h)`      | crossterm `Resize` event        | Cross-cutting: map state + render canvas |
| `Ui(SetTheme)`      | palette entry                   | Cross-cutting: UI theme + render styler |
| `Jump(LonLat)`      | search result, geoip plugin     | Plugin async → map state change        |
| `ActivatePlugin`    | keymap, palette                 | UI transition, same intent from 2 sources |
| `CycleFocus`        | Tab / Shift-Tab                 | UI transition                          |
| `OpenPalette`       | `:` key                         | UI transition                          |

### When to use a direct method / setter instead

Use a plain method call when **any** of the following is true:

1. It's a **completion notification** from a worker (render thread
   finished a frame; tile fetch finished downloading).
2. It's **periodic maintenance** (widget `poll()` tick, throttle timer).
3. It's **high-frequency** (many per second) and dispatch overhead
   would dominate.
4. Routing through the post-dispatch redraw rule would be **wrong**
   (infinite loop) or **pointless** (nothing else changes in response).

Current examples:

| Operation                         | Where                                    |
| --------------------------------- | ---------------------------------------- |
| `ui.drain_frames(&render_handle)` | Every tick, pulls completed MapFrames    |
| `ui.info.poll()`                  | Every tick, info overlay maintenance     |
| `widget.poll()`                   | Every tick, plugin async maintenance     |
| `render_handle.request_draw(…)`   | Sending to another thread (not app state) |

### The infinite-loop trap

Naively wrapping "frame arrived" as `Command::FrameArrived(frame)` is
tempting — everything goes through the same pipeline, right? It breaks:

```
frame arrives → Command::FrameArrived
             → dispatch → map_frame = Some(f)
             → InputEffect::Map → request_draw (post-dispatch rule)
             → render thread renders a frame
             → frame arrives → Command::FrameArrived
             → dispatch → map_frame = Some(f)
             → InputEffect::Map → request_draw
             → …
```

To avoid this you'd have to return `InputEffect::None` just for that
arm, which defeats the uniformity the pipeline buys you. A direct
`ui.drain_frames(…)` call sidesteps the rule entirely and is the right
shape.

### The decision question

When unsure, ask:

> "If this happens, should **other state also change** in response?"

- **YES** → probably a `Command`. The pipeline ensures the related
  changes fire consistently.
- **NO** → direct method. A `Command` just adds ceremony without
  earning anything.

## Controller split: by feature, not by domain

When `command.rs` grows past ~200 lines, split `dispatch` into
per-feature modules:

```
command/
  map_action.rs     # Map / Jump / Resize map-side
  resize.rs         # map + render_handle cross-cutting
  theme.rs          # ui + render_handle cross-cutting
  plugin.rs         # ActivatePlugin / CycleFocus
  palette.rs        # OpenPalette
```

Do **not** split by state domain (`map.rs` / `ui.rs` / `render.rs`).
Many commands are cross-cutting (e.g. `SetTheme` touches both UI theme
and render styler; `Resize` touches map state and render canvas). A
domain split forces these into arbitrary owners. A feature split lets
each module freely touch whatever state its feature needs.

Current threshold: not yet (command.rs is ~140 lines). Revisit when
adding commands pushes us over.

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
