# Configuration

ttymap is configured through a single Lua file:
`~/.config/ttymap/init.lua` (Neovim-style).

## Example

```lua
ttymap.opt.map.lat            = 35.6828
ttymap.opt.map.lon            = 139.7595
ttymap.opt.map.zoom           = 10
ttymap.opt.render.style       = "bright"   -- "dark" | "bright"
ttymap.opt.render.language    = "ja"

-- Per-plugin endpoint overrides live in their own Lua libs, not on
-- `ttymap.opt.*`. e.g. point the bundled `here` plugin at a private
-- IP-geolocation service:
require("ttymap.here").endpoint = "https://my-geoip.example.com/json"

ttymap.keymap.set("zoom_in", { "i", "+" })
ttymap.keymap.set("quit",    { "q", "C-q" })

-- Activate a user plugin (or re-activate a bundled one with custom
-- config seeded via a ttymap/<name>.lua holder lib).
require "myplug"

-- Conditional / computed config (the killer feature):
local heavy = os.getenv("TTYMAP_HEAVY") ~= nil
ttymap.opt.cache.memory_tiles = heavy and 2048 or 512
```

Bundled plugins are activated by `runtime/init.lua` (which runs
*before* your init.lua). To disable a bundled plugin, write your
own init.lua that lists only the plugins you want — see
`runtime/init.lua` for the default require list. Lua's
`package.loaded` cache makes a duplicate `require` from your
init.lua a no-op (registrations don't double-fire).

Every option is optional; omitted values stay at their built-in
defaults (see `runtime/init.lua` for the full schema with
defaults). Errors in `init.lua` (syntax, type mismatch, runtime
exception) are logged and recovered — the app keeps booting with
defaults.

## Options reference

The complete option tree lives in `runtime/init.lua` —
that file *is* the bundled defaults, and your
`~/.config/ttymap/init.lua` runs after it in the same Lua state
(last-wins on `ttymap.opt.*`). Browse it for the canonical list
with inline comments.

Top-level namespaces:

| Namespace | What it controls |
|---|---|
| `ttymap.opt.map` | initial lat/lon/zoom + zoom envelope |
| `ttymap.opt.render` | style preset, label language |
| `ttymap.opt.cache` | LRU size, on-disk persist toggle |
| `ttymap.opt.runtime` | poll/redraw cadence, sidebar width |
| `require("ttymap.<plugin>")` | per-plugin config holder libs (e.g. `ttymap.here.endpoint`) |
| `ttymap.keymap.set(action, keys)` | rebind a built-in action |
| `ttymap.keymap.del(action)` | drop a built-in binding |
| `require "<name>"` | activate a bundled or user plugin |

## Runtime path resolution

The binary builds an ordered list of runtime layers (Neovim-style
runtimepath). Higher layers shadow lower ones — drop a
`~/.config/ttymap/lua/plugin/wiki.lua` to replace bundled `wiki`.

1. `$TTYMAP_RUNTIME` — env override (escape hatch for hackers / CI / multiple checkouts)
2. `<workspace-root>/runtime` — `cargo run` from a git checkout (dev wins over stale install)
3. `$XDG_CONFIG_HOME/ttymap` (default `~/.config/ttymap`) — user overrides
4. `$XDG_DATA_HOME/ttymap` (default `~/.local/share/ttymap`) — `make install` target

A layer counts only when it has a `plugin/` or `lua/` subdirectory.
The user-tier is empty by default; you opt in by creating
`~/.config/ttymap/lua/plugin/` (require-able plugin modules — activate
by adding a `require "<name>"` to your init.lua) or
`~/.config/ttymap/lua/` (`require`'d shared libs).

## File locations

| Path | Content |
|------|---------|
| `~/.config/ttymap/init.lua` | Configuration |
| `~/.config/ttymap/lua/plugin/` | User Lua plugins (`*.lua` or `<name>/init.lua`); activate via `require "<name>"` in init.lua |
| `~/.config/ttymap/lua/` | User shared libs / overrides for bundled `lua/` scripts |
| `~/.local/share/ttymap/plugin/` | Bundled Lua plugins (placed by `make install`) |
| `~/.local/share/ttymap/lua/ttymap/` | Bundled shared libs (`fmt`, `sidebar`) |
| `~/.local/share/ttymap/exports/` | Frame exports (palette → "Export current frame") |
| `~/.cache/ttymap/` | Disk tile cache |
| `~/.local/state/ttymap/ttymap.log` | Log file (auto-rotated at 1 MB; only when `--log` is passed) |
