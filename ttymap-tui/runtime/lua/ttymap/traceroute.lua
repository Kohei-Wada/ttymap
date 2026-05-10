-- ttymap.traceroute — config holder for the bundled `traceroute` plugin.
--
-- Same nvim-style Lua-side seam as `ttymap.here`: this module returns one
-- cached table; the plugin reads it on every invocation, so init.lua
-- pre-pass mutations apply without restart:
--
--     -- ~/.config/ttymap/init.lua
--     local tr = require("ttymap.traceroute")
--     tr.line_color = 51              -- cyan instead of yellow
--     tr.seg_frames = 10              -- faster animation
--     tr.command    = "mtr -r -c 1 %s"  -- swap traceroute → mtr
--
-- `command` is a printf-style template; `%s` is replaced with the
-- requested host. The plugin validates the host against a strict
-- character whitelist before substitution, so the template stays
-- shell-safe as long as the host passes validation.
--
-- `geoip_url` / `geoip_parse` are paired — swapping one without the
-- other breaks resolution. Default targets `ip-api.com` (free, no key,
-- 45 req/min, returns `{status, lat, lon, ...}`).

return {
    -- Shell command template. %s ← host (validated upstream).
    command = "traceroute -n -q 1 -w 2 -m 20 %s",

    -- IP → geoip URL builder.
    geoip_url = function(ip) return "http://ip-api.com/json/" .. ip end,

    -- Body → (lon, lat) | nil. Returning nil marks the hop as
    -- geoip-failed (private range, lookup error, unparseable, …) —
    -- the plugin draws no marker for it.
    geoip_parse = function(body)
        local p = ttymap.json:parse(body)
        if not p or p.status ~= "success" then return nil end
        if type(p.lat) ~= "number" or type(p.lon) ~= "number" then
            return nil
        end
        return p.lon, p.lat
    end,

    -- Disk cache TTL for geoip lookups. IP→ASN mapping is stable
    -- for weeks; 24h is a conservative balance between correctness
    -- and not hammering the upstream during repeated traces.
    geoip_ttl_s = 86400,

    -- Frames to grow each segment between consecutive resolved hops.
    -- Effective frame rate is ~10 Hz (overlay_redraw_ms = 100), so
    -- 20 frames ≈ 2 s per segment.
    seg_frames = 20,

    -- Per-hop color resolver. Either an xterm-256 index (number, for
    -- a single-colour line) or a function `(hop_num) -> index` for a
    -- gradient / rainbow / per-AS scheme. Default cycles a cool
    -- cyan→magenta gradient so adjacent hops contrast while keeping
    -- the overall palette in the same family as ping_simulation.
    --
    -- The same value drives the polyline segment ending at hop N and
    -- the marker drawn at hop N — so the colour reads as "hop N's
    -- colour" all the way through.
    hop_color = function(hop_num)
        local palette = { 51, 45, 39, 33, 99, 135, 171, 207 }
        return palette[((hop_num - 1) % #palette) + 1]
    end,

    marker_glyph = "◎",
}
