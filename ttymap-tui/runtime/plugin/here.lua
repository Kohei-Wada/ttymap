-- here — palette action that jumps to the user's IP-geolocated
-- coordinates.
--
-- Windowless: nothing is pushed onto the compositor stack. The
-- palette `invoke` kicks a geoip GET if one isn't already in flight;
-- the per-frame `on_tick` callback polls the inflight job and, when
-- the response lands, hands the centre to `ttymap.animation.fly_to`
-- so the view glides over instead of teleporting (avoids the brief
-- black-tile gap from landing on un-prefetched coordinates).
--
-- The endpoint comes from `ttymap.config:geoip_endpoint()`.

local anim = require "ttymap.animation"

local state = { job = nil }
local tick_handle = nil  -- on_tick subscription while a fetch is in flight

local function drain(_map)
    if not state.job then return end
    local body = state.job:try_take()
    if body then
        local p = ttymap.json:parse(body)
        if p
            and type(p.latitude) == "number"
            and type(p.longitude) == "number" then
            anim.fly_to(p.longitude, p.latitude)
            ttymap.notify(string.format(
                "Flew to %.4f, %.4f", p.latitude, p.longitude
            ))
        else
            ttymap.notify("here: geoip response missing lat/lon",
                          { level = "warn" })
        end
        state.job = nil
        -- Job done; remove ourselves from the tick bus until the
        -- user invokes again. Avoids the per-frame `if not job` cost
        -- across the whole program lifetime.
        if tick_handle then
            tick_handle:remove()
            tick_handle = nil
        end
    end
end

ttymap.register_palette_command({
    label = "Jump to here (current location)",
    invoke = function()
        if not state.job then
            state.job = ttymap.http:fetch(ttymap.config:geoip_endpoint())
            if not tick_handle then
                tick_handle = ttymap.api.frame.on_tick(drain)
            end
        end
    end,
})
