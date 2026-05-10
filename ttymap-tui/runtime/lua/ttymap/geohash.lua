-- ttymap.geohash — config holder for the bundled `geohash` plugin.
--
-- nvim-style Lua-side seam (mirrors `ttymap.here` / `ttymap.traceroute`):
-- this module returns one cached table; the plugin reads it on every
-- invocation, so init.lua pre-pass mutations apply without restart.
--
--     -- ~/.config/ttymap/init.lua
--     local g = require("ttymap.geohash")
--     g.marker_glyph = "★"
--     g.marker_color = 220              -- xterm-256 yellow
--     g.djia_url     = "https://my-mirror.example.com/djia/%s/%s/%s"
--
-- `djia_url` is a printf template — `%s` slots are year, month
-- (zero-padded), day (zero-padded). Crox's standing service is the
-- de-facto provider for this exact role; any drop-in replacement
-- needs to return the opening DJIA value as a single line of plain
-- text (e.g. `42158.22\n`).

return {
    -- Crox geohashing DJIA service (returns plain-text opening value).
    djia_url = "http://geo.crox.net/djia/%s/%s/%s",

    -- DJIA opening for a given trading day never changes once
    -- published, so cache aggressively. 30 days is more than enough
    -- to cover repeated invocations across long ttymap sessions
    -- without re-asking Crox every time.
    djia_ttl_s = 86400 * 30,

    -- Marker style for the destination point. Map color args accept
    -- xterm-256 indices (0..255) or theme keywords ("accent",
    -- "accent_alt", "muted", "road").
    marker_glyph = "◇",
    marker_color = "accent_alt",
}
