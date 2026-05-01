# Lua plugin migration guide (legacy → nvim-style API)

The plugin API is now nvim-style: one identity declaration
(`register_plugin`) plus zero or more activation surfaces, and
**everything dynamic** (panels, palettes) is opened *imperatively*
from inside callbacks via `ttymap.api.*`.

## What's removed

| Old                                     | New                                                                   |
| --------------------------------------- | --------------------------------------------------------------------- |
| `register_overlay({ paint_on_map })`    | `register_plugin({ loop = function(map) ... end })`                   |
| `register_palette({ ... })` (top-level) | `ttymap.api.palette.open(spec)` from inside an `invoke` callback      |
| `ttymap.plugin:open()` / `:close()`     | `ttymap.api.window.open(spec)` returns a handle; close via `w:close()` |
| `ttymap.window:close()`                 | `w:close(); w = nil` from the plugin's own closure                    |
| `ttymap.window:export_frame()`          | `ttymap.api.frame.export()`                                           |
| `paint_on_map = function(map) ... end`  | painting moves into `loop`; same `map:point` / `map:text_anchored` API |
| `poll = function() ... end`             | async drain moves into `loop`                                         |

`register_plugin` itself is now optional — pure-action plugins
(e.g. `export.lua`) only declare a palette command.

## 1. Always-on chrome (no toggle)

Examples: `scalebar`, `info`, `attribution`. `paint_on_map` becomes
`loop`.

```lua
-- BEFORE: register_overlay({ name, paint_on_map = function(map) ... end })
-- AFTER:
ttymap.register_plugin({
    name = "scalebar",
    loop = function(map)
        map:text_anchored("bottom-right", 0, build_bar(map), "accent")
    end,
})
```

## 2. Toggle-able overlay (no window)

Example: `center` (crosshair). The loop runs every frame; visibility
is a plugin-internal flag the activation callback flips. Loop and
callback share the setup-state Lua VM, so the upvalue is visible
across them.

```lua
-- BEFORE: register_overlay({ paint_on_map }) + invoke = ttymap.plugin:open
-- AFTER:
local enabled = false
ttymap.register_plugin({
    name = "center",
    loop = function(map)
        if not enabled then return end
        local lon, lat = map:center()
        map:point(lon, lat, "+", "accent_alt")
    end,
})
local function toggle() enabled = not enabled end
ttymap.register_palette_command({ label = "Toggle center marker", invoke = toggle })
ttymap.register_keybind("c", toggle)
```

## 3. Toggle-able side panel (window)

Examples: `aircraft`, `quake`, `wiki`, `satellite`, `help`. Push the
panel imperatively; the returned handle's `:close()` pops it. Use the
handle itself as the "is open?" flag.

```lua
-- BEFORE: register_plugin({ paint_on_map, render, handle_event, poll })
--         + keybind invoke = ttymap.plugin:open
-- AFTER:
local w = nil  -- handle while open; nil while closed (also acts as enabled flag)
ttymap.register_plugin({
    name = "aircraft",
    loop = function(map)
        if not w then return end           -- panel closed → no work, no markers
        drain_inflight_fetch()             -- (was the old `poll`)
        for _, a in ipairs(state.aircraft) do
            map:point(a.lon, a.lat, marker(a), "accent")  -- (was `paint_on_map`)
        end
    end,
})
local function close() if w then w:close(); w = nil end end
local function open()
    if w then return end
    w = ttymap.api.window.open({
        layout = { anchor = "left", width = 56 },
        render = build_lines,
        handle_event = function(key) if key.code == "Esc" then close() end end,
    })
end
ttymap.register_keybind("a", function() if w then close() else open() end end)
```

## 4. One-shot palette action (no window)

Examples: `here`, `export`. The trivial case (`export`) is just a
palette command calling a `ttymap.api.*` primitive — no
`register_plugin` at all.

```lua
-- BEFORE: register_plugin({ poll = ttymap.window:export_frame })
--         + invoke = ttymap.plugin:open
-- AFTER:
ttymap.register_palette_command({
    label = "Export frame as ANSI",
    invoke = function() ttymap.api.frame.export() end,
})
```

For an async action (`here`: geoip → `ttymap.map:jump`), keep a
`register_plugin` whose `loop` drains the in-flight job; `invoke`
just kicks the fetch:

```lua
local state = { job = nil }
ttymap.register_plugin({
    name = "here",
    loop = function()
        if not state.job then return end
        local body = state.job:try_take()
        if body then
            local p = ttymap.json:parse(body)
            if p and p.latitude and p.longitude then
                ttymap.map:jump(p.longitude, p.latitude)
            end
            state.job = nil
        end
    end,
})
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
longer carries `poll`** — async drain moves into the plugin's `loop`.
`execute` self-closes via the captured handle.

```lua
-- BEFORE: register_palette({ prompt, submit_mode, filter, items,
--                            execute, poll, is_loading })
-- AFTER:
local w = nil  -- palette handle while open
ttymap.register_plugin({
    name = "search",
    loop = function() drain_inflight() end,  -- runs even when palette is closed
})
local function open()
    if w then return end
    w = ttymap.api.palette.open({
        prompt = "/", submit_mode = { kind = "debounced", ms = 400 },
        filter = function(q) ... end,
        items  = function() return state.items end,
        execute = function(idx)
            jump_to(idx)
            if w then w:close(); w = nil end
        end,
        is_loading = function() return state.pending end,
    })
end
ttymap.register_keybind("/", open)
ttymap.register_palette_command({ label = "Search location", invoke = open })
```

---

The bundled plugins under `runtime/plugin/` are reference
implementations of every category — copy the closest match.
