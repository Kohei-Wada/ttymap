# Lua subsystem

ttymap's plugin system is Lua-only. All in-tree plugins live in
`runtime/lua/plugin/`; the Rust side is the binding layer. `mlua` with
Lua 5.4 vendored.

## Plugin model

nvim-style. **A `.lua` file (or `<name>/init.lua` directory) under a
`<runtime>/lua/plugin/` layer is a require-able plugin module** — there
is no `register_plugin` ceremony. Activation is explicit: every plugin
is `require`-d from an `init.lua` file. The bundled `runtime/init.lua`
requires the default set; the user's `~/.config/ttymap/init.lua` may
add or override. Identity = the require name (also used for log tags
and `ttymap.help:palette_entries()`).

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

## Module layout (`ttymap-app/src/lua/`)

Split by intent, not by domain:

```
ttymap-app/src/lua/
  mod.rs           LuaSubsystem + merged build_subsystem (creates
                   the VM, installs API surface, runs the init.lua
                   chain — single entry point, no separate walker)
  vm.rs            new_lua + install_builtin_searcher (any
                   `<layer>/lua/<dot.path>.lua`, including
                   `plugin.<name>`) — standard `package.path`,
                   no custom plugin searcher
  tick.rs          dispatch_tick(bus, map) — per-frame fan-out of
                   the "tick" bucket on the host EventBus
  runtimepath.rs   runtime path resolution (env / manifest / xdg)
  init_lua.rs      run_init_lua_chain (system → user init.lua in
                   the shared VM); read_init_lua_config_only is
                   the snap-only thin path
  handle.rs        LuaHandle — host plumbing held by App
                   (drain_ops, tick, sync_view, set_current_frame,
                   set_attribution). Event publishing is not on
                   this surface — App accumulates events into a
                   buffer and publishes through the single
                   bus.publish call site in App::run.
  registrar.rs     LuaRegistry — live registry of activations +
                   palette entries; `register_*` calls push directly
                   here, Lua handles `:remove()` drop entries by ID
  map_api.rs       host-side MapApi struct (per-frame draw surface,
                   ratatui buffer + projection + theme; no mlua)
  host.rs          host-side Lua-runtime state (LuaHostShared,
                   LuaHostHandles, NotifyEntry, HelpEntry) — Rust
                   structs the api/ namespaces read/write
  api/             Rust→Lua API binding (the `ttymap` global).
                   File ↔ Lua-namespace 1:1 (file name = `ttymap.<X>`).
    mod.rs         install() — assembles the namespace userdatas;
                   also installs top-level `ttymap.notify` /
                   `ttymap.on_event` on the global
    http.rs        ttymap.http   :fetch / :fetch_cached / :url_encode
    json.rs        ttymap.json   :parse / :stringify
                                  (lua_to_json shared with storage.rs)
    sgp4.rs        ttymap.sgp4   TLE parse / propagate
    map.rs         ttymap.map    HostMap userdata (jump/zoom/fly_to/center)
                                + make_map_table (per-frame on_tick `map` arg)
    config.rs      ttymap.config (currently empty userdata; endpoint
                                   config moved to per-plugin Lua libs
                                   at runtime/lua/ttymap/<name>.lua)
    help.rs        ttymap.help   :keymap_entries / :palette_entries
    log.rs         ttymap.log    :info / :warn / :error
    tile.rs        ttymap.tile   :attribution
    storage.rs     ttymap.storage :open(ns) -> Store userdata
                                  (:get(k, default) / :set(k, v) / :delete(k);
                                   JSON-on-disk under XDG data dir; atomic
                                   write; corrupt / missing → default)
    register.rs    `ttymap.register_*` / `on_event` — every call pushes
                   directly into the live PluginRegistry / EventBus and
                   returns a Lua-facing handle (no deferred capture)
    imperative.rs  `ttymap.api.{card,palette,frame}` runtime cluster
  bridge/          Lua→Rust trait adapters
    handle.rs           shared dispatch plumbing
    card_component.rs   LuaCardComponent: Component for a Lua spec
    palette_provider.rs LuaPaletteProvider: PaletteProvider for a spec
    card_handle.rs      CardHandle + CloseFlag + CloseFlagWrapper
    card_parse.rs       Lua-table → CardSpec parsing helpers
    palette_handle.rs   PaletteHandle (mirror of CardHandle)
    event_handle.rs     EventHandle returned by on_event / on_tick
    registrar_handle.rs PaletteCommandHandle / KeybindHandle —
                        :remove() drops the matching entry from the
                        live PluginRegistry by ID
```

The host-side pub/sub registry that Lua subscribers attach to is
**`crate::event::EventBus`** (`ttymap-app/src/event/bus.rs`) — it's a
plain main-thread bus shared with non-Lua subscribers, not a Lua-only
type. `lua::tick::dispatch_tick` drains the `"tick"` bucket once per
frame; `EventBus::publish(Event::*)` drives every other event.

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
- **`EventBus`** (`crate::event::EventBus`, `ttymap-app/src/event/bus.rs`)
  — pub/sub registry keyed by event name. Every
  `ttymap.on_event(name, fn)` (and its `on_tick` sugar) call lands as
  a subscriber under that name. `lua::tick::dispatch_tick(bus, map)`
  runs the `"tick"` bucket against a live `MapApi` once per frame;
  `EventBus::publish(Event::*)` runs any other bucket. Errors from one
  callback are logged + swallowed so a single broken plugin can't
  freeze the host. The bus is shared with non-Lua subscribers.
- **`LuaHostShared`** (`lua/host.rs`) — runtime-data carrier
  (attribution, geoip endpoint, keymap entries, help-cheatsheet
  entries), Arc-cloned into each namespace userdata.
- **`PluginRegistry`** (`lua/registrar.rs`) — live `Rc<RefCell<...>>`
  registry of activations + palette entries that `ttymap.register_*`
  push directly into. `BaseLayer` borrows it on each keypress; the
  `:` palette-installer borrows it on each open; Lua-side
  `KeybindHandle:remove()` / `PaletteCommandHandle:remove()`
  mutably borrow it to drop entries by ID.

## Plugin runtime API (`ttymap` global)

Built by `ttymap::install` (`ttymap-app/src/lua/api/mod.rs`). Domain-namespaced
userdatas:

| Namespace        | Methods                                                                                        |
|------------------|------------------------------------------------------------------------------------------------|
| `ttymap.http`    | `:fetch(url)`, `:fetch_cached(url, ttl_secs)`, `:url_encode(s)`                                |
| `ttymap.map`     | `:jump(lon, lat)`, `:zoom(level)`, `:zoom()` getter, `:fly_to(lon, lat, zoom)`, `:center()`    |
| `ttymap.json`    | `:parse(s)` → table or nil, `:stringify(value)` → string                                       |
| `ttymap.sgp4`    | `:parse_tle`, `:parse_tles`, `:propagate`, `:propagate_batch`                                  |
| `ttymap.tile`    | `:attribution()`                                                                               |
| `ttymap.config`  | (currently empty userdata)                                                                     |
| `ttymap.help`    | `:keymap_entries()`, `:palette_entries()`                                                      |
| `ttymap.log`     | `:info(msg)`, `:warn(msg)`, `:error(msg)` — forward to host log at target `lua`                |
| `ttymap.storage` | `:open(namespace)` → `Store`; `Store:get(key, default)`, `:set(key, v)`, `:delete(key)`        |

### Activation surfaces

Top-level functions the script's setup body calls. Each returns a
disposable handle whose `:remove()` (`:close()` for the
compositor-stack openers) drops that registration — VS-Code-style
unified shape so a single `ttymap.plugin` Lua wrapper can manage
plugin lifetime uniformly.

- `ttymap.on_event(name, fn) -> EventHandle` — subscribe to a host
  event by name. See the [Event bus](#event-bus) for the canonical
  event-name set. `:remove()` drops the subscriber from the bus.
- `ttymap.api.frame.on_tick(fn) -> EventHandle` — sugar for
  `ttymap.on_event("tick", fn)`. `:remove()` drops the subscriber.
- `ttymap.register_palette_command({ label, invoke, hint }) -> PaletteCommandHandle`
  — adds a palette row whose `invoke` callback runs in the shared
  Lua state when selected. `:remove()` drops the entry from the
  live `PluginRegistry`; the next palette open won't list it.
- `ttymap.register_keybind(key, callback) -> KeybindHandle` —
  single-char top-level keybind. `:remove()` drops the activation
  from the live `PluginRegistry`; the next keypress for that key
  falls through to the keymap as if the plugin had never bound it.

All four handles share the disposable shape so a single
`ttymap.plugin` Lua wrapper can manage activate/deactivate
uniformly. Removal is idempotent — calling `:remove()` on an
already-dropped handle is a no-op. If the palette is open with an
entry visible at the moment a plugin removes it, selecting that
entry afterwards silently no-ops (the `CommandSeed`'s id-based
lookup against the registry returns `None`).

### Event bus

`ttymap.on_event(name, fn)` registers a callback against the host
`EventBus` (`crate::event::EventBus` — `ttymap-app/src/event/bus.rs`).
The per-frame `"tick"` bucket runs through `lua::tick::dispatch_tick`
(which threads a live `MapApi` through `Lua::scope`); every other
bucket runs through `EventBus::publish(Event::*)` with a typed
`Event` payload. The string an emit binds to is `Event::name()`
(`ttymap-app/src/event/payload.rs`).

| Name              | Fired                                                 | Payload              |
|-------------------|-------------------------------------------------------|----------------------|
| `tick`            | once per frame inside `ui::draw`                      | `MapApi` table       |
| `frame_ready`     | render thread produced a fresh `MapFrame`             | none                 |
| `map_jumped`      | `MapAction::Jump` ran                                 | `(lon, lat)`         |
| `map_zoom_set`    | `MapAction::SetZoom` ran                              | `zoom: number`       |
| `map_flew_to`     | `MapAction::FlyTo` ran                                | `(lon, lat, zoom)`   |
| `theme_changed`   | `UserCommand::SetTheme` ran                            | `theme: string`      |
| `resized`         | `UserCommand::Resize` ran                              | `(cols, rows)`       |
| `notify`          | `ttymap.notify(...)` or any host notify call           | `{ msg, level }`     |

Subscribers under different names are independent. One broken
subscriber doesn't stop the others — errors are logged and the
dispatch loop continues.

### Imperative primitives (`ttymap.api`)

Called from inside callbacks (palette invoke, keybind, on_tick):

- **`ttymap.api.card.open(spec) -> CardHandle`** — push a focused
  side-panel `LuaCardComponent` onto the compositor stack. Spec
  carries `layout`, `render`, `handle_key`, `footer_hints`, `name`.
- **`ttymap.api.palette.open(spec) -> PaletteHandle`** — push a
  palette-mode component. Spec carries `prompt`, `submit_mode`,
  `filter`, `items`, `execute`, `cancel`, `is_loading`. (No `poll` —
  async drain belongs in `on_tick`.) `cancel` fires on Esc and
  Enter+empty (default closes); `submit_mode` controls when `filter`
  fires — `"on_each_key"` (default), `{ kind = "debounced", ms = N }`,
  or `"on_enter"` (filter only on Enter+empty).
- **`ttymap.api.frame.to_ansi() -> string|nil`** — return the latest
  `MapFrame` rendered as an ANSI string, or `nil` if no frame has
  arrived yet. Producers (today: bundled `export.lua`) decide where +
  how to persist it.
- **`ttymap.api.frame.on_tick(fn)`** — subscribe a per-frame callback
  receiving a `MapApi` table. Stacks across calls.

`ttymap.notify(msg [, opts])` is a top-level function on the
`ttymap` global (not under `ttymap.api`) — it enqueues a `notify`
event on the host bus. The bundled `ttymap.notify` lib
(`runtime/lua/ttymap/notify.lua`, wired up by
`runtime/init.lua`'s `require("ttymap.notify").setup()`) subscribes
via `ttymap.on_event("notify", ...)` and renders recent entries
top-left. Pass `setup({ ttl_s = …, ring_cap = …,
max_text_width = … })` to override the renderer knobs; skipping
the call disables the toast renderer entirely (the bus event still
fires for any other subscriber).

### MapApi (per-frame drawing)

Bridged via a per-frame Lua table built inside `Lua::scope`
(`make_map_table` in `ttymap-app/src/lua/api/map.rs`) over the host-side
`MapApi` struct (`ttymap-app/src/lua/map_api.rs`). Methods: `point`, `label`, `text_anchored`,
`polyline`, `center`, `zoom`, `area_width`, `cursor`. Each `on_tick`
callback receives this table. **All drawing for non-window plugins
happens here.**

## Single shared Lua state

ttymap runs **one** Lua VM for the whole subsystem (Neovim-style).
`init.lua` runs in it first; then every bundled and user plugin
loads in the same state; then every callback for the program's
lifetime fires in the same state:

- every `on_tick` callback runs there,
- every palette `invoke` / keybind `callback` runs there,
- any window / palette `spec` is built there,
- `init.lua`'s `require "ttymap.<name>"` and a plugin's
  `require "ttymap.<name>"` resolve to **the same cached table**, so
  init.lua can mutate a config holder before the plugin loads and
  the plugin reads the mutated value when it loads.

Because every plugin shares one Lua state, **plugin-local upvalues**
(a `state` table, a `w` window-handle reference) are visible across
the plugin's own `register_*` callbacks. The toggle pattern works
exactly because the closures share the same module-level locals:

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

### Per-plugin config from `init.lua`

The single-VM design enables the Neovim-style override pattern: a
plugin exports a config holder under `<runtime>/lua/ttymap/<name>.lua`,
the plugin reads it via `require`, and `init.lua` mutates it via
`require`:

```lua
-- runtime/lua/ttymap/export.lua  (config holder)
return { dir = nil }   -- nil → built-in default

-- runtime/lua/plugin/export.lua  (plugin reads it)
local cfg = require "ttymap.export"
local function destination()
    return cfg.dir or os.getenv("PWD") or "."
end
ttymap.register_palette_command({
    label = "Export frame as ANSI",
    invoke = function()
        local ansi = ttymap.api.frame.to_ansi()
        if not ansi then return end
        local path = destination() .. "/ttymap.ans"
        local f = io.open(path, "w"); f:write(ansi); f:close()
    end,
})

-- ~/.config/ttymap/init.lua  (user override)
require("ttymap.export").dir = "/tmp/maps"
```

The user touches one line in `init.lua` instead of forking the whole
plugin (the old whole-file shadow pattern is still available but is
now reserved for the rare case of a totally different plugin
implementation).

## Drain pattern (host ↔ plugin)

Lua side is fire-and-forget; App drains per tick. Plugin-emitted
intent flows through a single typed buffer — the `OpsBuffer` shared
with each api/ namespace — carrying `Op::Push`, `Op::Close`,
`Op::Command(UserCommand)`, and `Op::Publish(Event)` variants
(`compositor/op.rs`). Examples:

- `ttymap.map:jump(lon, lat)` → `Op::Command(UserCommand::Map(MapAction::Jump))`
- `ttymap.map:zoom(level)` setter → `Op::Command(UserCommand::Map(MapAction::SetZoom))`
- `ttymap.map:fly_to(...)` → `Op::Command(UserCommand::Map(MapAction::FlyTo))`
- `ttymap.api.card.open(spec)` / `ttymap.api.palette.open(spec)`
  → `Op::Push { id, component }`
- `ttymap.notify(msg, opts)` → `Op::Publish(Event::Notify { ... })`

`LuaHandle::drain_ops` (`lua/handle.rs`) hands the App the queued
`Op`s once per loop iteration; `App::apply_ops` applies them
through the compositor and the dispatch entry point.

`map:polyline` overlays use a separate sink: `App.overlay.sink:
Vec<UserPolyline>` (`app/overlay.rs`) is borrowed by the per-frame
`MapApi`, plugins push during `on_tick`, App drains the sink
immediately after `ui::draw` into the next
`RenderTask::Draw { viewport, overlays }`. Render thread paints
overlays in a third pass after symbols (subpixel granularity preserved
via OR-merge into existing braille cells).

## Live host-state read-back

`ttymap.map:center()` and `ttymap.map:zoom()` (no-arg getter) read
shared `Arc<Mutex<...>>` cells. The host refreshes them once per
loop iteration in `App::drain_lua_host_handles`, before draining the
shared op buffer. That means the values are correct in **every**
callback path — palette `invoke`, `register_keybind` callbacks,
`on_tick` — not just inside an active window's dispatch. Cells are
shared with `HostMap` userdata via the same Arc.

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

Plugins resolve as **plain Lua modules** under
`<layer>/lua/plugin/<name>.lua` (or
`<layer>/lua/plugin/<name>/init.lua`). They share the same
`package.path` mechanism as any other lib — bundled `runtime/init.lua`
activates each by name with `require "plugin.<name>"`. There is no
custom searcher, no host-side plugin attribution: the chunk runs
once, `register_*` calls inside push directly into the live
registry, and Lua's `package.loaded` cache makes a duplicate
`require` from user init.lua a no-op.

The Rust side knows neither the `plugin/` directory convention nor
the searcher logic — it only owns the `ttymap.runtime_path`
primitive (used to extend `package.path` for every layer).
`package.path` carries `<layer>/lua/` only.

### `lua/plugin/` vs `lua/ttymap/`

- `<layer>/lua/plugin/<name>.lua` or
  `<layer>/lua/plugin/<name>/init.lua` — a plugin: a `.lua` file
  whose top level calls `register_palette_command` /
  `register_keybind` / `on_event` to add itself to the runtime.
  `require "plugin.<name>"` from init.lua activates it. Sub-modules
  resolve as `require "plugin.<name>.<sub>"`.
- `<layer>/lua/ttymap/<name>.lua` — a lib: a `.lua` file that
  returns a module table for callers to use (`ttymap.fmt`,
  `ttymap.notify`, `ttymap.user_config`). No `register_*` side
  effects.

Both are reachable from the same `package.path` — the convention
is purely organisational. A "plugin" can be moved under `ttymap/`
to be a lib (or vice versa) without any host-side changes.

### `make install`

Runs `cargo install --path .`, then copies `runtime/lua/` and
`runtime/init.lua` under `~/.local/share/ttymap/`. `cargo install`
alone fails fast with a "did you make install?" message.

## Config (`init.lua` chain) — also the plugin entry point

Rust runs **only** the bundled `<bundled-tier>/init.lua`. That file
is in charge of when (and whether) to load the user's init.lua
— Lua-side ordering policy, no Rust involvement. Rust does not
even name the user-config path: that's owned by a Lua lib,
`ttymap.user_config`, at `runtime/lua/ttymap/user_config.lua`.

By the time bundled init.lua runs, the API surface is fully
installed: `build_subsystem` does `ttymap.opt` / `ttymap.keymap`
pre-pass, then `api::install` adds `http` / `map` / `api` /
`register_*` / `notify` / `on_event`, then the plugin-aware
`package.searchers` entry is inserted. Then the bundled init.lua
runs and drives the rest.

The bundled init.lua's job (`ttymap-app/runtime/init.lua`) — standard
layered order (system → bundled → user):

1. Seed `ttymap.opt.*` with bundled defaults (mostly redundant with
   Rust seeds; serves as the documented schema).
2. `require` the bundled plugin set — they register before user
   config sees the registry.
3. `require("ttymap.user_config").load()` — resolves the user
   init.lua path (`$XDG_CONFIG_HOME/ttymap/init.lua` or
   `$HOME/.config/ttymap/init.lua`) and `dofile`s it.

User init.lua can then:

- mutate `ttymap.opt.*` (last-wins on the shared table)
- call `ttymap.keymap.set/del`
- `require "<plugin>"` to activate user plugins (their
  registrations stack on top of bundled in the registry)
- call `:remove()` on handles returned by their own
  `register_palette_command` / `register_keybind` / `on_event` to
  drop registrations later

Disabling a bundled plugin from user init.lua is intentionally
not part of this flow (bundled has already registered by the time
user runs). To opt out of the bundled set wholesale, point
`$TTYMAP_RUNTIME` at a custom runtime layer with your own
`init.lua`.

`ttymap.opt.*` exposes:

- **`opt.*` leaves** — pre-populated table tree seeded from Rust
  defaults. User mutates leaves.
- **`keymap.set(...)`** / **`keymap.del(...)`** — backed by a shared
  `KeybindingOverrides` map.

To replace the bundled defaults wholesale (e.g. fork the plugin set),
set `$TTYMAP_RUNTIME` to your own runtime layer with its own
`init.lua`.

Errors at any layer are logged + recovered; the host keeps booting.

## Bundled plugins (`runtime/lua/plugin/`)

18 total, all `require`-d from `runtime/init.lua`. Each plugin is a
reference implementation of one shape —
always-on chrome (`attribution`, `scalebar`, `help`), toggleable
overlay (`center`, `here`, `ping_simulation`, `terminator`,
`autospin`), toggleable side panel (`info`, `aircraft`, `satellite`,
`wiki`, `travel`), palette one-shot (`export`, `quake`, `antipode`),
palette provider (`search`), or quick game (`geo_quiz`):

```
aircraft/        antipode.lua     attribution.lua    autospin.lua
center.lua       export.lua       geo_quiz.lua      help.lua
here.lua         info.lua         ping_simulation.lua  quake.lua
satellite/       scalebar.lua     search/            terminator.lua
travel/          wiki/
```

The toast renderer for `ttymap.notify(...)` lives as a lib at
`runtime/lua/ttymap/notify.lua`, not as a plugin — it's
infrastructure for every plugin's notify calls, with knobs that
naturally belong on a `setup({...})` call. `runtime/init.lua`
wires it up via `require("ttymap.notify").setup()`.

Directory plugins (`aircraft/`, `satellite/`, `search/`, `travel/`,
`wiki/`) use `<plugin>/init.lua` as the entry; sibling files load via
`require "<plugin>.<name>"`.

`satellite` is a **single multi-sat tracker** — one palette entry,
one panel showing every configured satellite (ISS + Hubble bundled,
plus whatever the user appends to `satellite/init.lua`'s spec list).
Per-sat key chars (`i` / `h` …) inside the panel toggle individual
visibility. TLE fetch (CelesTrak) and SGP4 propagation
(`ttymap.sgp4`) run per visible sat from inside `on_tick`.

`travel` packages curated multi-country itineraries (Japan + Italy
out of the box) and choreographs an animated tour through each
route's stops via `ttymap.director` — a perfect demonstration of
the Lua-side scriptable-scenes layer (see below).

## Shared libraries (`runtime/lua/ttymap/`)

`require`-only Lua modules (no plugin discovery, no `register_*`
side effects). Each is independently useful from any plugin or
`init.lua`. Keep the Rust bridge primitive — composition lives
here.

| Module                  | Surface                                                        |
| ----------------------- | -------------------------------------------------------------- |
| `ttymap.fmt`            | `.distance(meters)` — short human-readable distance string    |
| `ttymap.geo`            | `.haversine_m` / `.bearing_label` / `.antipode` — great-circle math helpers |
| `ttymap.sidebar`        | `.up_pressed` / `.down_pressed` / `.is_close_key` / `.cycle`  |
| `ttymap.animation`      | `.fly_to(lon, lat, zoom, opts?)` — frame-based pan animation  |
| `ttymap.director`       | `.run(fn, opts?)` / `.fly` / `.wait` / `.tween` — coroutine-based scheduler |
| `ttymap.cities`         | array of `{lon, lat, name, country}` — ~170 worldwide cities  |
| `ttymap.location`       | `.get(cb)` / `.refresh(cb)` / `.cached()` — shared IP-geoip user-location cache |

`ttymap.animation.fly_to` interpolates `ttymap.map:fly_to` over ~30
frames (default), with optional `on_done` / `on_cancel` callbacks
fired on natural completion / pre-emption. Cancellation: manual
user pan / zoom is detected by comparing live map state against
the value dispatched last frame (Braille cell tolerance). A fresh
`fly_to` over an in-flight one fires the previous `on_cancel` —
pre-emption semantics match manual input.

`ttymap.director` builds a procedural-looking async API on top of
animation + a frame timer:

```lua
local director = require "ttymap.director"
director.run(function()
    ttymap.notify("Starting tour")
    director.fly(139.69, 35.69, 10)         -- yields until arrival
    director.wait(120)                        -- yields 120 frames
    for _, stop in ipairs(stops) do
        director.fly(stop.lon, stop.lat, stop.zoom)
        ttymap.notify(stop.note)
        director.wait(120)
    end
end, { on_cancel = function() ... end })
```

Internally it's a coroutine + a directive enum (`fly` / `wait` /
`tween`) yielded back to a single `on_tick` driver. Multiple
`director.run` calls run in parallel — each registers its own
coroutine. Cancellation propagates from the animation lib's
`on_cancel` (manual input pre-empts the active `fly`), or from
explicit `handle:cancel()`. A natural function return ends the
script without firing `on_cancel`. The travel plugin's pre /
stop / post tour is one such script — the whole choreography is
top-to-bottom procedural Lua, no hand-written state machine.

`ttymap.location` resolves "where is the user, in lon/lat" once
via IP geoip and serves the answer to every plugin that asks.
Three layers: in-memory (this session), `ttymap.storage` (across
sessions, TTL-bounded — default 24h), and an HTTP fetch against
`require("ttymap.here").endpoint` (the canonical source, shared
with the `here` plugin). Concurrent `loc.get` calls coalesce onto
a single in-flight job; the cb fires sync on cache hit or once the
shared response lands. Errors surface as `cb(nil, nil, "error")`
plus a `ttymap.notify` warning. `loc.refresh(cb)` skips the TTL
and always re-fetches; `loc.cached()` is a sync read that returns
`nil, nil` on miss / stale and never starts a fetch.

## User plugins

Drop a `*.lua` file (or a `<plugin>/init.lua` directory) into
`~/.config/ttymap/lua/plugin/`, then add `require "<plugin>"` to
`~/.config/ttymap/init.lua` to activate it. Auto-discovery was
removed — the require makes activation explicit and gives you
control over load order. `~/.config/ttymap/lua/plugin/` files without
a corresponding require are silently ignored; the bootstrap logs
a one-shot warning to point that out.

The directory layout lets a large plugin spread its source across
`<plugin>/init.lua` + sibling files (`<plugin>/state.lua`, etc.)
reachable via `require "<plugin>.state"`. The plugin searcher
resolves both top-level and dotted names against `<layer>/lua/plugin/`;
top-level wraps with attribution, dotted is a plain chunk.

## Footer hints

`BaseLayer::footer_hints` shows core keymap shortcuts (`hjkl/a/z/:/q`)
plus a dynamically-derived list of plugin keybinds harvested from
`Registrar.palette_entries` at startup. Disabling or rebinding a
plugin updates the footer for free. **No plugin name is hardcoded in
`compositor/base.rs`.**

Per-window footer hints live inline in the
`ttymap.api.card.open(spec)` spec via
`footer_hints = { {key, label}, ... }`.
