-- satellite — TLE-driven trackers. One palette entry per satellite.
--
-- Each entry uses the shared `ttymap.satellites` factory: TLE fetch
-- (CelesTrak), SGP4 propagation via `ttymap.sgp4`, marker / label /
-- panel. Add another satellite by appending to `entries` (or by
-- dropping a similar file under `~/.config/ttymap/plugins/`).
--
-- Per-entry `name` drives compositor dedup, so distinct satellites
-- coexist on the stack — toggle ISS and Hubble both ON simultaneously.

local make = require("ttymap.satellites").make

return {
    name = "satellite",
    entries = {
        make({ display = "ISS",    norad_id = 25544, color = "accent_alt" }),
        make({ display = "Hubble", norad_id = 20580, color = "accent" }),
    },
}
