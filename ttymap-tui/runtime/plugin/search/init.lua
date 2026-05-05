-- search (Lua port) — Nominatim forward geocoding as a palette
-- provider, opened on `/` or "Search location" palette command.
--
-- Two halves:
--   * `ttymap.api.frame.on_tick` runs per-frame and drains the
--     in-flight HTTP job (the palette spec carries no `poll` field;
--     async drain belongs in the plugin's tick callback).
--   * `open()` calls `ttymap.api.palette.open(spec)` which pushes a
--     `PaletteComponent` onto the compositor. The palette closes
--     itself on Enter / Esc — `execute` + `cancel` both return to
--     the host's `apply_action`, which calls `win.close()`. No
--     plugin-side handle bookkeeping needed.
--
-- The Nominatim REST client (URL builder + result parser) lives in
-- `search.nominatim`.

local nominatim = require("search.nominatim")

local state = {
    job = nil,
    pending = false,
    last_query = "",
    candidates = {},  -- list of { name, lon, lat }
}

-- Per-frame async drain. Runs whether or not the palette is open —
-- the in-flight job outlives a re-open if the user dismisses and
-- reopens the palette mid-fetch (state is module-scoped).
ttymap.api.frame.on_tick(function()
    if state.job then
        local body = state.job:try_take()
        if body then
            local payload = ttymap.json:parse(body)
            if not payload then
                -- Short-circuit so we don't follow up with a
                -- misleading `No results for "X"` info popup —
                -- there may well be results, we just couldn't read
                -- the response.
                ttymap.notify("search: Nominatim response unparseable",
                              { level = "warn" })
                state.pending = false
                state.job = nil
                return
            end
            state.candidates = nominatim.parse(payload)
            if #state.candidates == 0 and state.last_query ~= "" then
                ttymap.notify(string.format(
                    "No results for \"%s\"", state.last_query
                ))
            end
            state.pending = false
            state.job = nil
        end
    end
end)

local function open()
    ttymap.api.palette.open({
        prompt = "/",
        -- Enter triggers the fetch; typing buffers silently. Keeps
        -- the upstream API quiet and gives the user a clear "submit"
        -- moment — no surprise requests while they're still typing.
        submit_mode = "on_enter",

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
            state.job = ttymap.http:fetch(nominatim.url(trimmed))
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
        end,

        is_loading = function() return state.pending end,
    })
end

ttymap.register_keybind("/", open)
ttymap.register_palette_command({ label = "Search location", invoke = open })
