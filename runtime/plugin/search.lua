-- search (Lua port) — Nominatim forward geocoding as a palette
-- provider, opened on `/` or "Search location" palette command.
--
-- Two halves:
--   * `ttymap.api.frame.on_tick` runs per-frame and drains the
--     in-flight HTTP job (the palette spec carries no `poll` field;
--     async drain belongs in the plugin's tick callback).
--   * `open()` calls `ttymap.api.palette.open(spec)` which pushes a
--     `PaletteComponent` onto the compositor and hands back a handle.
--     `execute` self-closes via `w:close(); w = nil` so the next
--     activation pushes a fresh palette.
--
-- Debounce 400 ms keeps Nominatim's free endpoint from being
-- hammered while the user types.

local SEARCH_URL = "https://nominatim.openstreetmap.org/search"
local LIMIT = 5
local DEBOUNCE_MS = 400

local state = {
    job = nil,
    pending = false,
    last_query = "",
    candidates = {},  -- list of { name, lon, lat }
}
local w = nil  -- palette handle while open; nil while closed

local function search_url(query)
    return string.format("%s?q=%s&format=json&limit=%d",
        SEARCH_URL, ttymap.http:url_encode(query), LIMIT)
end

local function parse_results(payload)
    local out = {}
    if type(payload) ~= "table" then return out end
    for _, item in ipairs(payload) do
        if type(item) == "table"
            and type(item.display_name) == "string"
            and type(item.lat) == "string"
            and type(item.lon) == "string" then
            local lat = tonumber(item.lat)
            local lon = tonumber(item.lon)
            if lat and lon then
                table.insert(out, {
                    name = item.display_name,
                    lon = lon,
                    lat = lat,
                })
            end
        end
    end
    return out
end

-- Per-frame async drain. Runs whether or not the palette is open —
-- the in-flight job outlives a re-open if the user dismisses and
-- reopens the palette mid-fetch (state is module-scoped).
ttymap.api.frame.on_tick(function()
    if state.job then
        local body = state.job:try_take()
        if body then
            state.candidates = parse_results(ttymap.json:parse(body))
            state.pending = false
            state.job = nil
        end
    end
end)

local function open()
    if w then return end
    w = ttymap.api.palette.open({
        prompt = "/",
        submit_mode = { kind = "debounced", ms = DEBOUNCE_MS },

        filter = function(query)
            local trimmed = query:match("^%s*(.-)%s*$") or ""
            if trimmed == "" then
                state.candidates = {}
                state.pending = false
                state.last_query = ""
                return
            end
            if trimmed == state.last_query
                and (state.pending or #state.candidates > 0) then
                return
            end
            state.last_query = trimmed
            state.candidates = {}
            state.job = ttymap.http:fetch(search_url(trimmed))
            state.pending = true
        end,

        items = function()
            local out = {}
            for _, c in ipairs(state.candidates) do
                table.insert(out, { label = c.name, hint = "" })
            end
            return out
        end,

        execute = function(idx)
            local c = state.candidates[idx]
            if c then ttymap.map:jump(c.lon, c.lat) end
            if w then
                w:close()
                w = nil
            end
        end,

        is_loading = function() return state.pending end,
    })
end

ttymap.register_keybind("/", open)
ttymap.register_palette_command({ label = "Search location", invoke = open })
