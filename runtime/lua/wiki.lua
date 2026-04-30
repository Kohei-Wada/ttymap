-- wiki (Lua port) — Wikipedia geosearch panel + map markers.
--
-- Two-stage fetch: first the geosearch endpoint returns titles +
-- coordinates + distance for nearby articles, then a second call to
-- the extracts endpoint pulls a 5-sentence summary per title. We
-- chain them via a small state machine because each `ttymap.http:fetch`
-- handle is a single in-flight job; the original Rust impl did both
-- requests synchronously inside one worker.
--
-- The list/detail mode toggle lives in `state.detail` — when set,
-- `render` shows the full extract for that one article and key
-- handling switches to "Esc/Backspace/Enter back, Up/Down browse".
-- A refresh on 'r' or on (re)open replaces `articles` while
-- preserving overlap by title; selection clamps if the list shrinks.

local fmt = require "ttymap.fmt"

local LANGUAGE = "en"
local LIMIT = 50

local state = {
    articles = {},
    selected = 1,           -- 1-based index into articles
    detail = nil,           -- snapshot of the article being read
    phase = "idle",         -- "idle" | "geosearching" | "extracting"
    job = nil,              -- in-flight ttymap.http:fetch handle
    pending_pages = nil,    -- titles + coords + dist between geosearch and extracts
    needs_refresh = false,  -- true on (re)open and on 'r'
}

local function geosearch_url(lat, lon)
    return string.format(
        "https://%s.wikipedia.org/w/api.php?action=query&list=geosearch"
        .. "&gscoord=%f|%f&gsradius=10000&gslimit=%d&format=json",
        LANGUAGE, lat, lon, LIMIT
    )
end

local function extracts_url(titles)
    -- Wikipedia accepts pipe-separated titles. Encode each title
    -- individually (ttymap.http:url_encode handles spaces / non-ASCII /
    -- reserved chars) and join with a literal `|`, which the API
    -- treats as the separator. For our ~50-entry limit the URL
    -- fits well under the host's path length limits.
    local encoded = {}
    for _, t in ipairs(titles) do
        table.insert(encoded, ttymap.http:url_encode(t))
    end
    return string.format(
        "https://%s.wikipedia.org/w/api.php?action=query&prop=extracts"
        .. "&exintro=1&explaintext=1&exsentences=5&titles=%s&format=json",
        LANGUAGE, table.concat(encoded, "|")
    )
end

local function parse_geosearch(payload)
    local out = {}
    if not payload or not payload.query then return out end
    local arr = payload.query.geosearch
    if type(arr) ~= "table" then return out end
    for _, p in ipairs(arr) do
        if type(p.title) == "string"
            and type(p.lat) == "number"
            and type(p.lon) == "number"
            and type(p.dist) == "number" then
            table.insert(out, {
                title = p.title,
                lat = p.lat,
                lon = p.lon,
                dist_m = p.dist,
            })
        end
    end
    return out
end

local function parse_extracts(payload)
    local out = {}
    if not payload or not payload.query or type(payload.query.pages) ~= "table" then
        return out
    end
    for _, page in pairs(payload.query.pages) do
        if type(page.title) == "string" then
            out[page.title] = page.extract or ""
        end
    end
    return out
end

local function merge_articles(new_articles)
    -- Keep articles still in the new list, append new ones, clamp
    -- the selection. Mirrors `WikiState::set_articles` from the Rust
    -- impl so a refresh doesn't yank the user off their current pick.
    local present = {}
    for _, a in ipairs(new_articles) do present[a.title] = true end
    local kept = {}
    for _, a in ipairs(state.articles) do
        if present[a.title] then table.insert(kept, a) end
    end
    local existing = {}
    for _, a in ipairs(kept) do existing[a.title] = true end
    for _, a in ipairs(new_articles) do
        if not existing[a.title] then table.insert(kept, a) end
    end
    state.articles = kept
    if state.selected > #state.articles then
        state.selected = math.max(1, #state.articles)
    end
end

local function start_refresh()
    local lon, lat = ttymap.map:center()
    state.phase = "geosearching"
    state.job = ttymap.http:fetch(geosearch_url(lat, lon))
end

local function step_state_machine()
    if not state.job then return end
    local body = state.job:try_take()
    if not body then return end

    if state.phase == "geosearching" then
        local pages = parse_geosearch(ttymap.json:parse(body))
        state.job = nil
        if #pages == 0 then
            state.phase = "idle"
            merge_articles({})
            return
        end
        local titles = {}
        for _, p in ipairs(pages) do table.insert(titles, p.title) end
        state.pending_pages = pages
        state.phase = "extracting"
        state.job = ttymap.http:fetch(extracts_url(titles))
    elseif state.phase == "extracting" then
        local extracts = parse_extracts(ttymap.json:parse(body))
        local merged = {}
        for _, p in ipairs(state.pending_pages or {}) do
            table.insert(merged, {
                title = p.title,
                lat = p.lat,
                lon = p.lon,
                dist_m = p.dist_m,
                extract = extracts[p.title] or "",
            })
        end
        state.pending_pages = nil
        state.job = nil
        state.phase = "idle"
        merge_articles(merged)
    end
end

local function move_selection(direction)
    local n = #state.articles
    if n == 0 then return end
    if direction == -1 then
        state.selected = state.selected > 1 and state.selected - 1 or n
    else
        state.selected = state.selected < n and state.selected + 1 or 1
    end
    local a = state.articles[state.selected]
    if a then ttymap.map:jump(a.lon, a.lat) end
end

return {
    label = "Toggle wiki",
    key = "i",
    layout = { anchor = "right", width = 56 },

    footer_hints = {
        { "C-n/C-p", "select" },
        { "Enter",   "open" },
        { "Esc",     "back" },
        { "r",       "refresh" },
        { "i",       "close wiki" },
    },

    render = function()
        if state.detail then
            local d = state.detail
            local lines = {
                {{ text = d.title, style = "highlight" }},
                {{ text = fmt.distance(d.dist_m) .. "  ", style = "muted" }},
            }
            if d.extract and #d.extract > 0 then
                table.insert(lines, "")
                for line in d.extract:gmatch("[^\r\n]+") do
                    table.insert(lines, line)
                end
            else
                table.insert(lines, {{ text = "(no summary available)", style = "muted" }})
            end
            return lines
        end

        if #state.articles == 0 then
            return { { { text = "Loading...", style = "muted" } } }
        end

        local lines = {}
        for i, a in ipairs(state.articles) do
            local title_style = (i == state.selected) and "highlight" or "accent"
            table.insert(lines, {
                { text = a.title,                              style = title_style },
                { text = "  " .. fmt.distance(a.dist_m),    style = "muted" },
            })
        end
        return lines
    end,

    paint_on_map = function(map)
        for i, a in ipairs(state.articles) do
            local color = (i == state.selected) and "accent_alt" or "accent"
            map:point(a.lon, a.lat, "●", color)
        end
    end,

    handle_event = function(key)
        local code = key.code
        local ch = key.char
        local ctrl = key.ctrl

        -- Self-toggle on the activation key.
        if code == "Char" and ch == "i" and not ctrl then
            return { close = true }
        end

        -- Refresh always available.
        if code == "Char" and ch == "r" and not ctrl then
            state.needs_refresh = true
            return nil
        end

        local up   = (ctrl and code == "Char" and ch == "p") or code == "Up"
        local down = (ctrl and code == "Char" and ch == "n") or code == "Down"
        local exit_detail = code == "Esc" or code == "Backspace" or code == "Enter"

        if state.detail then
            if exit_detail then
                state.detail = nil
                return nil
            end
            if up or down then
                move_selection(up and -1 or 1)
                local a = state.articles[state.selected]
                if a then state.detail = a end
            end
            return nil
        end

        if #state.articles == 0 then
            -- Pre-data: still consume widget-control keys so they
            -- don't fall through, but let everything else pass.
            if up or down or exit_detail then return nil end
            return { ignore = true }
        end

        if code == "Enter" then
            local a = state.articles[state.selected]
            if a then
                state.detail = a
                ttymap.map:jump(a.lon, a.lat)
            end
            return nil
        end
        if up or down then
            move_selection(up and -1 or 1)
            return nil
        end
        if code == "Esc" or code == "Backspace" then
            return nil
        end
        return { ignore = true }
    end,

    poll = function()
        if state.needs_refresh and not state.job then
            state.needs_refresh = false
            start_refresh()
        end
        step_state_machine()
        -- Initial fetch after first poll: if articles are empty and
        -- we haven't kicked off a job yet, do so now.
        if #state.articles == 0 and not state.job and state.phase == "idle" then
            start_refresh()
        end
    end,
}
