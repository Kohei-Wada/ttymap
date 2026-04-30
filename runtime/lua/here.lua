-- here — palette action that jumps to the user's IP-geolocated
-- coordinates.
--
-- Headless: pushed onto the compositor stack on palette select,
-- fires a single geoip GET on first poll, emits `ttymap.map:jump(...)`
-- when the response arrives, then self-closes via
-- `ttymap.window:close()`. The component itself paints nothing;
-- render / paint_on_map are omitted so the map underneath stays
-- untouched.
--
-- The endpoint comes from `ttymap.config:geoip_endpoint()`, which
-- reads `[geoip] endpoint` from `config.toml`.

local state = {
    job = nil,
    started = false,
    done = false,
}

ttymap.register_plugin({
    name = "here",
    label = "Jump to here (current location)",

    handle_event = function(_)
        -- Non-modal: never consume keys. Lets the user keep panning
        -- while the lookup runs (typically <1s).
        return { ignore = true }
    end,

    poll = function()
        if state.done then return end

        if not state.started then
            state.started = true
            state.job = ttymap.http:fetch(ttymap.config:geoip_endpoint())
            return
        end

        if state.job then
            local body = state.job:try_take()
            if body then
                state.job = nil
                local payload = ttymap.json:parse(body)
                if payload
                    and type(payload.latitude) == "number"
                    and type(payload.longitude) == "number" then
                    ttymap.map:jump(payload.longitude, payload.latitude)
                end
                state.done = true
                ttymap.window:close()
            end
        end
    end,
})
