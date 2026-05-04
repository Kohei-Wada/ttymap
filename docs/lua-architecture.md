# Lua subsystem

ttymap's plugin system is Lua-only. All in-tree plugins live in
`runtime/plugin/`; the Rust side is the binding layer. `mlua` with
Lua 5.4 vendored.

This doc is the architectural reference. For the user-facing "how do
I write a plugin" walkthrough, see
[lua-plugin-migration.md](lua-plugin-migration.md).

## Plugin model

nvim-style. **A `.lua` file in a `<runtime>/plugin/` directory is a
plugin** — there is no `register_plugin` ceremony. Identity = file
stem (used for log tags, stem-dedup across runtime layers, and
`ttymap.opt.disable` matching).

A script joins host loops by calling some combination of:

- `ttymap.on_event(name, fn)` — generic pub/sub subscription; the
  callback fires once per host event with event-specific args
  (see [Event bus](#event-bus) below)
- `ttymap.api.frame.on_tick(fn)` — sugar for
  `ttymap.on_event("tick", fn)`; per-frame work, multiple calls per
  script stack
- `register_palette_command({ label, invoke })` — palette row
- `register_keybind(key, callback)` — top-level keybind

At least one of those is required at script load. Everything dynamic
(panels, palettes) is *imperative*, opened from inside callbacks via
`ttymap.api.card.open` / `ttymap.api.palette.open`.

## Module layout (`ttymap-tui/src/lua/`)

Split by intent, not by domain:

```
ttymap-tui/src/lua/
  mod.rs           discovery, register_one, package.searchers wiring
  registry.rs      LuaEventBus — pub/sub for tick / frame_ready / map_jumped / …
  runtimepath.rs   runtime path resolution (env / manifest / xdg)
  init_lua.rs      separate config-DSL Lua state (opt + keymap)
  handle.rs        shared host handle plumbing (LuaHandle)
  registrar.rs     Registrar — collects activations / palette entries
  sender.rs        channel sender helpers (push to App via crossbeam)
  map_api.rs       host-side MapApi struct (per-frame draw surface,
                   ratatui buffer + projection + theme; no mlua)
  host.rs          host-side Lua-runtime state (LuaHostShared,
                   LuaHostHandles, NotifyEntry, PluginEntry) — Rust
                   structs the api/ namespaces read/write
  capture.rs       receptacle types for plugin → host registration
                   (PaletteCommandSpec, KeybindSpec, EventSubscription,
                   CapturedRegistration, CaptureSlot)
  api/             Rust→Lua API binding (the `ttymap` global).
                   File ↔ Lua-namespace 1:1 (file name = `ttymap.<X>`).
    mod.rs         install() — assembles the namespace userdatas
    http.rs        ttymap.http   :fetch / :url_encode
    json.rs        ttymap.json   :parse
    sgp4.rs        ttymap.sgp4   TLE parse / propagate
    map.rs         ttymap.map    HostMap userdata (jump/zoom/fly_to/center)
                                + make_map_table (per-frame on_tick `map` arg)
    config.rs      ttymap.config :geoip_endpoint
    help.rs        ttymap.help   :keymap_entries / :palette_entries
    log.rs         ttymap.log    :info / :warn / :error
    tile.rs        ttymap.tile   :attribution
    register.rs    setup-time `ttymap.register_*` / `on_event` capture
    imperative.rs  `ttymap.api.{card,palette,frame,notify}` runtime cluster
  bridge/          Lua→Rust trait adapters
    handle.rs           shared dispatch plumbing (LuaHandle)
    card_component.rs   LuaCardComponent: Component for a Lua spec
    palette_provider.rs LuaPaletteProvider: PaletteProvider for a spec
    card_handle.rs      CardHandle + CloseFlag + CloseFlagWrapper
    card_parse.rs       Lua-table → CardSpec parsing helpers
    palette_handle.rs   PaletteHandle (mirror of CardHandle)
```

## Key Rust types

- **`LuaCardComponent`** (`bridge/card_component.rs`) — implements
  `Component` by dispatching to a Lua spec table. One per
  `ttymap.api.card.open(spec)` call.
- **`LuaPaletteProvider`** (`bridge/palette_provider.rs`) — same idea
  for `PaletteProvider`. Built by `from_spec`.
- **`CardHandle`** / **`PaletteHandle`** — userdata returned to Lua
  by `api.card.open` / `api.palette.open`. `:close()` flips a shared
  `CloseFlag` that the host polls each frame to pop the component
  (idempotent).
- **`CloseFlagWrapper`** (`bridge/card_handle.rs`) — a `Component`
  decorator that observes a `CloseFlag` from outside the inner
  component (used for palette providers, which have no `poll`).
- **`LuaHandle`** (`bridge/handle.rs`) — shared dispatch plumbing for
  both window and palette callback paths.
- **`LuaEventBus`** (`registry.rs`) — pub/sub registry keyed by
  event name. Every `ttymap.on_event(name, fn)` (and its
  `on_tick` sugar) call lands as a `Subscriber` under that name.
  `dispatch_tick(map)` runs the `"tick"` bucket against a live
  `MapApi`; `dispatch(name, args)` runs any other bucket. Errors
  from one callback are logged + swallowed so a single broken
  plugin can't freeze the host.
- **`LuaHostShared`** (`ttymap/mod.rs`) — runtime-data carrier
  (attribution, geoip endpoint, keymap entries, palette entries),
  Arc-cloned into each namespace userdata.

## Plugin runtime API (`ttymap` global)

Built by `ttymap::install` (`ttymap-tui/src/lua/api/mod.rs`). Domain-namespaced
userdatas:

| Namespace        | Methods                                                                                        |
|------------------|------------------------------------------------------------------------------------------------|
| `ttymap.http`    | `:fetch(url)`, `:fetch_cached(url, ttl)`, `:url_encode(s)`                                     |
| `ttymap.map`     | `:jump(lon, lat)`, `:zoom(level)`, `:zoom()` getter, `:fly_to(lon, lat, zoom)`, `:center()`    |
| `ttymap.json`    | `:parse(s)` → table or nil                                                                     |
| `ttymap.sgp4`    | `:parse_tle`, `:parse_tles`, `:propagate`, `:propagate_batch`                                  |
| `ttymap.tile`    | `:attribution()`                                                                               |
| `ttymap.config`  | `:geoip_endpoint()`                                                                            |
| `ttymap.help`    | `:keymap_entries()`, `:palette_entries()`                                                      |
| `ttymap.log`     | `:info(msg)`, `:warn(msg)`, `:error(msg)` — forward to host log at target `lua[<plugin>]`      |

### Activation surfaces

Top-level functions the script's setup body calls:

- `ttymap.on_event(name, fn)` — subscribe to a host event by name.
  See the [Event bus](#event-bus) for the canonical event-name set.
- `ttymap.api.frame.on_tick(fn)` — sugar for
  `ttymap.on_event("tick", fn)`.
- `ttymap.register_palette_command({ label, invoke, hint })` — adds a
  palette row whose `invoke` callback runs in the setup state when
  selected.
- `ttymap.register_keybind(key, callback)` — single-char top-level
  keybind. Callback runs in the setup state.

### Event bus

`ttymap.on_event(name, fn)` registers a callback against the
`LuaEventBus` (`ttymap-tui/src/lua/registry.rs`). Every emit site inside the
host calls `LuaEventBus::dispatch_tick` (for the per-frame `"tick"`
bucket, which threads a live `MapApi` through `Lua::scope`) or
`LuaEventBus::dispatch(name, args)` (for plain-Lua-value payloads).

Canonical event names (`names::*` in `registry.rs`):

| Name              | Fired                                                 | Payload              |
|-------------------|-------------------------------------------------------|----------------------|
| `tick`            | once per frame inside `ui::draw`                      | `MapApi` table       |
| `frame_ready`     | render thread produced a fresh `MapFrame`             | none                 |
| `map_jumped`      | `MapAction::Jump` ran                                 | `(lon, lat)`         |
| `map_zoom_set`    | `MapAction::SetZoom` ran                              | `zoom: number`       |
| `map_flew_to`     | `MapAction::FlyTo` ran                                | `(lon, lat, zoom)`   |
| `theme_changed`   | `UserCommand::SetTheme` ran                            | `theme: string`      |
| `resized`         | `UserCommand::Resize` ran                              | `(cols, rows)`       |
| `frame_exported`  | `UserCommand::ExportFrame` ran                         | none                 |

Subscribers under different names are independent. One broken
subscriber doesn't stop the others — errors are logged and the
dispatch loop continues.

### Imperative primitives (`ttymap.api`)

Called from inside callbacks (palette invoke, keybind, on_tick):

- **`ttymap.api.card.open(spec) -> CardHandle`** — push a focused
  side-panel `LuaCardComponent` onto the compositor stack. Spec
  carries `layout`, `render`, `handle_event`, `footer_hints`, `name`.
- **`ttymap.api.palette.open(spec) -> PaletteHandle`** — push a
  palette-mode component. Spec carries `prompt`, `submit_mode`,
  `filter`, `items`, `execute`, `cancel`, `is_loading`. (No `poll` —
  async drain belongs in `on_tick`.) `cancel` fires on Esc and
  Enter+empty (default closes); `submit_mode` controls when `filter`
  fires — `"on_each_key"` (default), `{ kind = "debounced", ms = N }`,
  or `"on_enter"` (filter only on Enter+empty).
- **`ttymap.api.frame.export()`** — fire-and-forget snapshot to disk.
- **`ttymap.api.frame.on_tick(fn)`** — subscribe a per-frame callback
  receiving a `MapApi` table. Stacks across calls.
- **`ttymap.notify(msg [, opts])`** — post a transient status message;
  the bundled `notify` plugin renders recent entries top-right.
  `ttymap.api.notify.recent(ttl_ms)` is the read side.

### MapApi (per-frame drawing)

Bridged via a per-frame Lua table built inside `Lua::scope`
(`make_map_table` in `ttymap-tui/src/lua/api/map.rs`) over the host-side
`MapApi` struct (`ttymap-tui/src/lua/map_api.rs`). Methods: `point`, `label`, `text_anchored`,
`polyline`, `center`, `zoom`, `area_width`, `cursor`. Each `on_tick`
callback receives this table. **All drawing for non-window plugins
happens here.**

## Setup state and callback execution

Each plugin file runs in its own persistent **setup state** — the
Lua VM that ran the script's top-level `register_*` calls. That same
VM is also where:

- every `on_tick` callback runs,
- every palette `invoke` / keybind `callback` runs,
- any window / palette `spec` is built.

Because they share one Lua state, **plugin-local upvalues** (a
`state` table, a `w` window-handle reference) are visible across all
of them. That's how the toggle pattern works:

```lua
local w = nil
ttymap.register_keybind("i", function()
  if w then w:close(); w = nil
  else w = ttymap.api.card.open(spec)
  end
end)
```

Component visibility / multi-instance / one-shot self-closing are all
**plugin-side policy decisions**. Rust just gives you `open` (returns
a handle) and `:close()` (idempotent flip).

## Drain pattern (host ↔ plugin)

Lua side is fire-and-forget; App drains per tick. Senders held by the
setup state:

- `jump_tx` — `ttymap.map:jump` → `UserCommand::Map(MapAction::Jump)`
- `zoom_tx` — `ttymap.map:zoom(level)` setter → `MapAction::SetZoom`
- `fly_to_tx` — `ttymap.map:fly_to` → `MapAction::FlyTo`
- `export_tx` — `ttymap.api.frame.export` → `UserCommand::ExportFrame`
- `push_tx` — `Box<dyn Component>` queued by `api.card.open` /
  `api.palette.open` → pushed onto the compositor stack

Receivers live on `LuaHostHandles` (returned from `install`); App
walks every plugin's handles in `drain_lua_host_handles` once per
loop iteration.

The same drain-on-each-iteration pattern carries `map:polyline`
overlays: `App.overlay_sink: Vec<UserPolyline>` is borrowed by the
per-frame `MapApi`, plugins push during `on_tick`, App drains the
sink immediately after `ui::draw` into the next
`RenderTask::Draw { viewport, overlays }`. Render thread paints
overlays in a third pass after symbols (subpixel granularity preserved
via OR-merge into existing braille cells).

## Live host-state read-back

`ttymap.map:center()` and `ttymap.map:zoom()` (no-arg getter) read
shared `Arc<Mutex<...>>` cells. The host refreshes them once per
loop iteration in `App::drain_lua_host_handles`, before draining each
plugin's setup-state channels. That means the values are correct in
**every** callback path — palette `invoke`, `register_keybind`
callbacks, `on_tick` — not just inside an active window's dispatch.
Cells are shared with `HostMap` userdata via the same Arc.

## Runtime path resolution

Neovim-style ordered list (`runtimepath.rs`). Every layer with a
`plugin/` or `lua/` subdir counts:

1. `$TTYMAP_RUNTIME` (env override)
2. `$CARGO_MANIFEST_DIR/runtime` (dev — wins over stale install)
3. `$XDG_CONFIG_HOME/ttymap` (user)
4. `$XDG_DATA_HOME/ttymap` (bundled — `~/.local/share/ttymap`)

Layer 2 path is the maintainer's home dir baked at compile time; on a
user machine it doesn't exist and is filtered out, so user > bundled
in production.

`register_builtin_plugins` walks the list with **stem dedup** — so
`~/.config/ttymap/plugin/wiki.lua` shadows the bundled `wiki`. The
`package.searchers` and `package.path` extensions also walk every
layer.

### `plugin/` vs `lua/`

- `<layer>/plugin/*.lua` — auto-discovered plugins.
- `<layer>/lua/<dot.path>.lua` — `require`'d shared libs only
  (e.g. `ttymap.fmt`). Auto-discovery never touches `lua/`.

Resolution for the `lua/` tier is a custom `package.searchers` entry
that walks every runtime layer, mirroring Neovim's runtime-path
searcher.

### `make install`

Runs `cargo install --path .`, then copies `runtime/plugin/`,
`runtime/lua/`, and `runtime/init.lua` under
`~/.local/share/ttymap/`. `cargo install` alone fails fast with a
"did you make install?" message.

## Config (`init.lua` chain)

Two `init.lua` files run in the same Lua state — last-wins on
`ttymap.opt.*` and `ttymap.keymap`:

1. `<bundled-tier>/init.lua` (first hit among env / manifest / xdg_data)
2. `~/.config/ttymap/init.lua`

Note: this is a **separate Lua state** from the plugin runtime state
(`init_lua.rs`). The same `ttymap` global name is reused for `opt` /
`keymap` (config DSL) — no collision because the scopes don't share a
VM.

`load_init_lua` exposes:

- **`opt.*`** — pre-populated table tree seeded from Rust defaults.
  User mutates leaves.
- **`keymap.set(...)`** / **`keymap.del(...)`** — backed by a shared
  `KeybindingOverrides` map.
- **`opt.disable = { "wiki", "quake" }`** — skips matching plugins at
  registration time (stem-match).

Errors at any layer are logged + recovered; the chain keeps walking.

## Bundled plugins (`runtime/plugin/`)

14 total, each a reference implementation of one of the migration-guide
categories (always-on chrome, toggleable overlay, toggleable side
panel, palette one-shot, palette provider):

```
aircraft/        attribution.lua  center.lua    export.lua
help.lua         here.lua         info.lua      notify.lua
ping_simulation  quake.lua        satellite/    scalebar.lua
search/          wiki/
```

Directory plugins (`aircraft/`, `satellite/`, `search/`, `wiki/`) use
`<plugin>/init.lua` as the entry; sibling files load via
`require "<plugin>.<name>"`.

`satellite` is a **single multi-sat tracker** — one palette entry,
one panel showing every configured satellite (ISS + Hubble bundled,
plus whatever the user appends to `satellite/init.lua`'s spec list).
Per-sat key chars (`i` / `h` …) inside the panel toggle individual
visibility. TLE fetch (CelesTrak) and SGP4 propagation
(`ttymap.sgp4`) run per visible sat from inside `on_tick`.

## User plugins

Drop a `*.lua` file (or a `<plugin>/init.lua` directory) into
`~/.config/ttymap/plugin/`. Same walker, same lifecycle.

The directory layout lets a large plugin spread its source across
`<plugin>/init.lua` + sibling files (`<plugin>/state.lua`, etc.)
reachable via `require "<plugin>.state"` through the extended
`package.path`.

## Footer hints

`BaseLayer::footer_hints` shows core keymap shortcuts (`hjkl/a/z/:/q`)
plus a dynamically-derived list of plugin keybinds harvested from
`Registrar.palette_entries` at startup. Disabling or rebinding a
plugin updates the footer for free. **No plugin name is hardcoded in
`compositor/base.rs`.**

Per-window footer hints live inline in the
`ttymap.api.card.open(spec)` spec via
`footer_hints = { {key, label}, ... }`.

## Migration

For before/after examples covering all five plugin shapes
(always-on chrome, toggleable overlay, toggleable side panel, palette
one-shot, palette provider), see
[lua-plugin-migration.md](lua-plugin-migration.md).
