-- search (Lua port) — Nominatim forward geocoding as a palette
-- provider.
--
-- Non-Component shape: this script returns a PaletteProvider-style
-- table — the Rust [`LuaPaletteProvider`] adapter wires it into the
-- universal-picker. `/` (and the "Search location" palette entry)
-- pushes a fresh palette pre-loaded with this provider.
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

return {
    name = "search",
    key = "/",
    label = "Search location",

    -- Presence of `palette` makes this script a palette provider.
    -- The dispatcher reads `kind` from the shape, not a field.
    palette = {
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
            if trimmed == state.last_query and (state.pending or #state.candidates > 0) then
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
            if c then
                ttymap.map:jump(c.lon, c.lat)
                return nil
            end
            return { close = true }
        end,

        poll = function()
            if state.job then
                local body = state.job:try_take()
                if body then
                    state.candidates = parse_results(ttymap.json:parse(body))
                    state.pending = false
                    state.job = nil
                end
            end
        end,

        is_loading = function()
            return state.pending
        end,
    },
}
