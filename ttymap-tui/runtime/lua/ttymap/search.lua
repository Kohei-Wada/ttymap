-- ttymap.search — config holder for the bundled `search` plugin.
--
-- Lua-side seam for users who want a different forward-geocoder
-- endpoint (private Nominatim, alternative provider with the same
-- response shape, etc.) without shadowing the whole plugin file.
--
-- nvim-style: this module returns one cached table; the plugin
-- requires the same table on activation, so any pre-pass mutation
-- in init.lua is visible:
--
--     -- ~/.config/ttymap/init.lua
--     require("ttymap.search").endpoint = "https://my-nominatim/search"
--
-- The bundled plugin reads `endpoint` on every fetch, so the
-- override applies immediately on next query — no restart needed.
--
-- Default endpoint is the public OSM Nominatim instance, which is
-- rate-limited to 1 rps by `ttymap-engine`'s shared HTTP client per
-- the OSM Foundation usage policy
-- (https://operations.osmfoundation.org/policies/nominatim/).
-- Pointing at a self-hosted endpoint falls through the rate-limit
-- registry on the Rust side (host suffix doesn't match
-- `nominatim.openstreetmap.org`) and runs unthrottled.

return {
    endpoint = "https://nominatim.openstreetmap.org/search",
}
