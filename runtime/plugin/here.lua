-- here — palette action that jumps to the user's IP-geolocated
-- coordinates.
--
-- Windowless: nothing is pushed onto the compositor stack. The
-- palette `invoke` kicks a geoip GET if one isn't already in flight;
-- the per-frame `loop` polls the inflight job and, when the response
-- lands, fires `ttymap.map:jump(...)` (drained by App per A7) and
-- clears state.
--
-- The endpoint comes from `ttymap.config:geoip_endpoint()`, which
-- reads `[geoip] endpoint` from `config.toml`.

local state = { job = nil }

ttymap.register_plugin({
    name = "here",
    loop = function()
        if not state.job then return end
        local body = state.job:try_take()
        if body then
            local p = ttymap.json:parse(body)
            if p
                and type(p.latitude) == "number"
                and type(p.longitude) == "number" then
                ttymap.map:jump(p.longitude, p.latitude)
            end
            state.job = nil
        end
    end,
})

ttymap.register_palette_command({
    label = "Jump to here (current location)",
    invoke = function()
        if not state.job then
            state.job = ttymap.http:fetch(ttymap.config:geoip_endpoint())
        end
    end,
})
