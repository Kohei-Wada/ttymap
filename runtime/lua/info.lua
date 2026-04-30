-- info (Lua port) — top-right always-on chrome showing centre /
-- cursor / zoom / reverse-geocoded place name.
--
-- The reverse-geocode lookup hits Nominatim's free `/reverse`
-- endpoint with a 5 s throttle (the upstream asks callers to stay
-- under 1 req/s; 5 s is comfortably under). Throttle is plugin-side
-- because the rest of the painted readout updates every frame.

local NOMINATIM_URL = "https://nominatim.openstreetmap.org/reverse"
local INTERVAL_SEC = 5

local state = {
    place_name = nil,
    job = nil,
    last_query = nil,        -- "lat,lon" string for the in-flight or last fetch
    last_fetch_sec = 0,
}

local function reverse_url(lat, lon)
    return string.format("%s?lat=%f&lon=%f&format=json&zoom=10",
        NOMINATIM_URL, lat, lon)
end

local function format_place(payload)
    if not payload then return nil end
    local addr = payload.address
    local city
    if addr then
        city = addr.city or addr.town or addr.village
    end
    local country = addr and addr.country
    if city and country then
        return city .. ", " .. country
    elseif country then
        return country
    elseif city then
        return city
    elseif payload.display_name then
        return payload.display_name
    end
    return nil
end

local function refresh(lat, lon)
    -- Always-running plugin; only kick a new fetch when the throttle
    -- window has elapsed and no other request is in flight.
    if state.job then return end
    local now = os.time()
    if (now - state.last_fetch_sec) < INTERVAL_SEC then return end
    state.last_fetch_sec = now
    state.last_query = string.format("%.4f,%.4f", lat, lon)
    state.job = ttymap.http:fetch(reverse_url(lat, lon))
end

ttymap.register_plugin({
    name = "info",
    activation = "overlay",

    paint_on_map = function(map)
        local lon, lat = map:center()
        local zoom = map:zoom()

        map:text_anchored("top-right", 0,
            string.format(" center: %.3f, %.3f ", lat, lon), "accent")

        local clon, clat = map:cursor()
        local cursor_line
        if clon and clat then
            cursor_line = string.format(" cursor: %.3f, %.3f ", clat, clon)
        else
            cursor_line = " cursor: unknown "
        end
        map:text_anchored("top-right", 1, cursor_line, "accent")

        map:text_anchored("top-right", 2,
            string.format(" zoom: %.1f ", zoom), "accent")

        local place = state.place_name or "unknown"
        map:text_anchored("top-right", 3,
            " place: " .. place .. " ", "accent")
    end,

    poll = function()
        if state.job then
            local body = state.job:try_take()
            if body then
                local payload = ttymap.json:parse(body)
                state.place_name = format_place(payload)
                state.job = nil
            end
        end
        local lon, lat = ttymap.map:center()
        refresh(lat, lon)
    end,
})
