-- bookmark — save named map locations and fly back to them later.
--
-- A pure-Lua demonstration that opinionated navigation needs no new
-- Rust: it composes the existing primitives only —
-- `ttymap.map:center()` / `:zoom()` to read the camera,
-- `ttymap.animation.fly_to` to move it, and `ttymap.storage` for
-- cross-session persistence.
--
-- Surfaces:
--   * `m`                    → save the current view under a typed name
--   * palette "Save bookmark"   → same as `m`
--   * palette "Go to bookmark"  → pick a saved view, fly to it
--   * palette "Delete bookmark" → pick a saved view, forget it
--
-- On-disk shape: one storage key `"data"` under namespace
-- `"bookmark"` holding `{ [name] = { lon, lat, zoom, desc } }`. A
-- single key (rather than one file per bookmark) sidesteps the lack
-- of a key-enumeration API on `Store`.

local anim = require "ttymap.animation"

local STORE_NS = "bookmark"
local STORE_KEY = "data"

-- Resolved lazily: ttymap.storage is absent when no per-user data
-- dir is available (see api/storage.rs `new()`), in which case we
-- fall back to a session-only in-memory table.
local store = nil
local mem = {}

local function ensure_store()
    if store == nil and ttymap and ttymap.storage then
        store = ttymap.storage:open(STORE_NS)
    end
    return store
end

local function load()
    local s = ensure_store()
    if s then return s:get(STORE_KEY, {}) end
    return mem
end

local function save(all)
    local s = ensure_store()
    if s then s:set(STORE_KEY, all) else mem = all end
end

-- Sorted name list so the picker order is stable (hash-table
-- iteration order is not).
local function names(all)
    local out = {}
    for name in pairs(all) do out[#out + 1] = name end
    table.sort(out)
    return out
end

local function trim(s)
    return s:match("^%s*(.-)%s*$") or ""
end

-- Parse the save line `name :: description` (description optional)
-- into `name, desc`. Splitting on the first `::` keeps `::` usable
-- inside a description; a bare line is all name.
local function parse_save(line)
    local name, desc = line:match("^(.-)%s*::%s*(.*)$")
    if name then return trim(name), trim(desc) end
    return trim(line), ""
end

------------------------------------------------------------
-- Save: type `name :: description`, confirm with Enter. The
-- description is optional — a bare name works too.
------------------------------------------------------------
local function open_save()
    local name, desc = "", ""
    ttymap.api.palette.open({
        prompt = "save (name :: description)> ",
        submit_mode = "on_each_key",
        filter = function(query) name, desc = parse_save(query) end,
        items = function()
            if name == "" then return {} end
            local label = "Save \"" .. name .. "\""
            if desc ~= "" then label = label .. " — " .. desc end
            return { { label = label, hint = "" } }
        end,
        execute = function()
            if name == "" then return end
            local lon, lat = ttymap.map:center()
            local all = load()
            all[name] = {
                lon = lon, lat = lat, zoom = ttymap.map:zoom(),
                desc = desc ~= "" and desc or nil,
            }
            save(all)
            ttymap.notify(string.format("Bookmarked \"%s\"", name))
        end,
    })
end

------------------------------------------------------------
-- Go to / Delete: pick from the saved list.
------------------------------------------------------------
local function open_picker(opts)
    local all = load()
    local ordered = names(all)
    if #ordered == 0 then
        ttymap.notify("No bookmarks yet — press m to save one")
        return
    end

    local query = ""
    -- Names of the rows `items()` last returned, in display order, so
    -- `execute(idx)` indexes exactly what the user sees — no second
    -- filter pass that could diverge from the rendered list.
    local visible = {}
    ttymap.api.palette.open({
        prompt = opts.prompt,
        submit_mode = "on_each_key",
        filter = function(q) query = trim(q):lower() end,
        items = function()
            visible = {}
            local out = {}
            for _, name in ipairs(ordered) do
                if query == "" or name:lower():find(query, 1, true) then
                    local b = all[name]
                    visible[#visible + 1] = name
                    out[#out + 1] = {
                        label = name,
                        hint = b.desc or string.format("%.3f, %.3f", b.lat, b.lon),
                    }
                end
            end
            return out
        end,
        execute = function(idx)
            local name = visible[idx]
            if name and all[name] then opts.on_pick(name, all[name], all) end
        end,
    })
end

local function open_goto()
    open_picker({
        prompt = "go to> ",
        on_pick = function(name, b)
            anim.fly_to(b.lon, b.lat, b.zoom)
            ttymap.notify(string.format("Flew to \"%s\"", name))
        end,
    })
end

local function open_delete()
    open_picker({
        prompt = "delete> ",
        on_pick = function(name, _b, all)
            all[name] = nil
            save(all)
            ttymap.notify(string.format("Deleted \"%s\"", name))
        end,
    })
end

ttymap.register_keybind("m", open_save)
ttymap.register_palette_command({ label = "Save bookmark", hint = "m", invoke = open_save })
ttymap.register_palette_command({ label = "Go to bookmark", invoke = open_goto })
ttymap.register_palette_command({ label = "Delete bookmark", invoke = open_delete })
