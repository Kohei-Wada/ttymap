# Lua plugin migration guide (legacy → nvim-style API)

The plugin API is now nvim-style: a script's existence in
`<runtime>/plugin/` *is* the registration. There is no
`register_plugin` ceremony — identity = file stem (used for log
tags, `ttymap.opt.disable` matching, and `help.lua`'s metadata
surface). The script joins host loops by calling some combination
of `ttymap.api.frame.on_tick(fn)` (per-frame work; multiple calls
per script are stacked), `register_palette_command({label, invoke})`
(palette row), and `register_keybind(key, callback)` (top-level
keybind). At least one is required. Everything dynamic (panels,
palettes) is *imperative*, opened from inside callbacks via
`ttymap.api.*`.

## What's removed

| Old                                       | New                                                                            |
| ----------------------------------------- | ------------------------------------------------------------------------------ |
| `register_plugin({ name, loop })`         | `ttymap.api.frame.on_tick(function(map) ... end)` (no `name`)                  |
| `register_overlay({ paint_on_map })`      | `ttymap.api.frame.on_tick(function(map) ... end)`                              |
| `register_palette({ ... })` (top-level)   | `ttymap.api.palette.open(spec)` from inside an `invoke` callback               |
| `ttymap.plugin:open()` / `:close()`       | `ttymap.api.card.open(spec)` returns a handle; close via `w:close()`         |
| `ttymap.window:close()`                   | `w:close(); w = nil` from the plugin's own closure                             |
| `ttymap.window:export_frame()`            | `ttymap.api.frame.export()`                                                    |
| `paint_on_map = function(map) ... end`    | painting moves into `on_tick`; same `map:point` / `map:text_anchored` API      |
| `poll = function() ... end`               | async drain moves into `on_tick`                                               |
| `enabled = false` (in spec)               | early-`return` at top of script, OR add stem to `ttymap.opt.disable`           |
| `ttymap.opt.plugins.disable = {...}`      | `ttymap.opt.disable = {...}` (flat namespace)                                  |

Pure-action plugins (e.g. `export.lua`) only declare a palette
command. No `on_tick` needed.

## 1. Always-on chrome (no toggle)

Examples: `scalebar`, `info`, `attribution`. `paint_on_map` becomes
`on_tick`.

```lua
-- BEFORE: register_plugin({ name, loop = function(map) ... end })
-- AFTER:
ttymap.api.frame.on_tick(function(map)
    map:text_anchored("bottom-right", 0, build_bar(map), "accent")
end)
```

## 2. Toggle-able overlay (no window)

Example: `center` (crosshair). `on_tick` runs every frame;
visibility is a plugin-internal flag the activation callback flips.
Tick callback and the activation callback share the setup-state Lua
VM, so the upvalue is visible across them.

```lua
local enabled = false
ttymap.api.frame.on_tick(function(map)
    if not enabled then return end
    local lon, lat = map:center()
    map:point(lon, lat, "+", "accent_alt")
end)
local function toggle() enabled = not enabled end
ttymap.register_palette_command({ label = "Toggle center marker", invoke = toggle })
ttymap.register_keybind("c", toggle)
```

## 3. Toggle-able side panel (window)

Examples: `aircraft`, `quake`, `wiki`, `satellite`, `help`. Push the
panel imperatively; the returned handle's `:close()` pops it. Use the
handle itself as the "is open?" flag.

```lua
local w = nil  -- handle while open; nil while closed (also acts as enabled flag)
ttymap.api.frame.on_tick(function(map)
    if not w then return end                  -- panel closed → no work, no markers
    drain_inflight_fetch()                    -- (was the old `poll`)
    for _, a in ipairs(state.aircraft) do
        map:point(a.lon, a.lat, marker(a), "accent")  -- (was `paint_on_map`)
    end
end)
local function close() if w then w:close(); w = nil end end
local function open()
    if w then return end
    w = ttymap.api.card.open({
        render = build_lines,
        handle_key = function(key) if key.code == "Esc" then close() end end,
    })
end
ttymap.register_keybind("a", function() if w then close() else open() end end)
```

## 4. One-shot palette action (no window)

Examples: `here`, `export`. The trivial case (`export`) is just a
palette command calling a `ttymap.api.*` primitive — no `on_tick` at
all.

```lua
ttymap.register_palette_command({
    label = "Export frame as ANSI",
    invoke = function() ttymap.api.frame.export() end,
})
```

For an async action (`here`: geoip → `ttymap.map:jump`), keep an
`on_tick` callback to drain the in-flight job; `invoke` just kicks
the fetch:

```lua
local state = { job = nil }
ttymap.api.frame.on_tick(function()
    if not state.job then return end
    local body = state.job:try_take()
    if body then
        local p = ttymap.json:parse(body)
        if p and p.latitude and p.longitude then
            ttymap.map:jump(p.longitude, p.latitude)
        end
        state.job = nil
    end
end)
ttymap.register_palette_command({
    label  = "Jump to here",
    invoke = function()
        if not state.job then
            state.job = ttymap.http:fetch(ttymap.config:geoip_endpoint())
        end
    end,
})
```

## 5. Palette provider

Example: `search` (Nominatim forward-geocode). `register_palette` at
top level is gone. Push the provider with
`ttymap.api.palette.open(spec)` from inside `invoke`. The spec **no
longer carries `poll`** — async drain moves into `on_tick`. The
palette closes itself on Enter / Esc; `execute` runs on Enter+item,
`cancel` runs on Esc / Enter+empty for any cleanup the plugin needs.

```lua
ttymap.api.frame.on_tick(function() drain_inflight() end)  -- runs even when palette is closed
local function open()
    ttymap.api.palette.open({
        prompt = "/", submit_mode = "on_enter",  -- or { kind = "debounced", ms = 400 }
        filter = function(q) ... end,
        items  = function() return state.items end,
        execute = function(idx) jump_to(idx) end,
        -- cancel = function() ... end,  -- optional: fires on Esc / Enter+empty
        is_loading = function() return state.pending end,
    })
end
ttymap.register_keybind("/", open)
ttymap.register_palette_command({ label = "Search location", invoke = open })
```

---

The bundled plugins under `runtime/plugin/` are reference
implementations of every category — copy the closest match.
