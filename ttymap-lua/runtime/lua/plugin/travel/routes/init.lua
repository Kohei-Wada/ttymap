-- travel.routes — manifest of all bundled country route packs.
--
-- Each entry is a country table `{ country, routes }` returned from a
-- sibling file. Adding a new country is a two-step:
--
--   1. Drop a `<country>.lua` next to this file with the same shape
--      as `japan.lua`.
--   2. Append a `(require "plugin.travel.routes.<country>")` line to the
--      list below — note the parentheses, they are load-bearing
--      (Lua 5.4 `require` returns the module *and* loader info; the
--      parens truncate to a single value so the list doesn't grow
--      phantom string entries from the trailing position).
--
-- We don't auto-discover via filesystem walk — Lua has no portable
-- `readdir`, and the explicit list keeps load order deterministic.

return {
    (require "plugin.travel.routes.japan"),
    (require "plugin.travel.routes.italy"),
}
