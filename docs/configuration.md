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

-- IP-based geolocation (shared by `--here` flag and the `here` plugin)
ttymap.opt.geoip.on_startup   = false
ttymap.opt.geoip.endpoint     = "https://ipapi.co/json/"
ttymap.opt.geoip.timeout_ms   = 2000

ttymap.keymap.set("zoom_in", { "i", "+" })
ttymap.keymap.set("quit",    { "q", "C-q" })

-- Disable bundled plugins by stem
table.insert(ttymap.opt.disable, "wiki")

-- Conditional / computed config (the killer feature):
local heavy = os.getenv("TTYMAP_HEAVY") ~= nil
ttymap.opt.cache.memory_tiles = heavy and 2048 or 512
```

Every option is optional; omitted values stay at their built-in
defaults (see `ttymap-tui/runtime/init.lua` for the full schema with
defaults). Errors in `init.lua` (syntax, type mismatch, runtime
exception) are logged and recovered — the app keeps booting with
defaults.

## Options reference

The complete option tree lives in `ttymap-tui/runtime/init.lua` —
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
| `ttymap.opt.geoip` | endpoint, timeout, on-startup behaviour |
| `ttymap.opt.runtime` | poll/redraw cadence, sidebar width |
| `ttymap.opt.disable` | list of plugin stems to skip |
| `ttymap.keymap.set(action, keys)` | rebind a built-in action |
| `ttymap.keymap.del(action)` | drop a built-in binding |

## Runtime path resolution

The binary builds an ordered list of runtime layers (Neovim-style
runtimepath). Higher layers shadow lower ones — drop a
`~/.config/ttymap/plugin/wiki.lua` to replace bundled `wiki`.

1. `$TTYMAP_RUNTIME` — env override (escape hatch for hackers / CI / multiple checkouts)
2. `$CARGO_MANIFEST_DIR/runtime` — `cargo run` from a git checkout (dev wins over stale install)
3. `$XDG_CONFIG_HOME/ttymap` (default `~/.config/ttymap`) — user overrides
4. `$XDG_DATA_HOME/ttymap` (default `~/.local/share/ttymap`) — `make install` target

A layer counts only when it has a `plugin/` or `lua/` subdirectory.
The user-tier is empty by default; you opt in by creating
`~/.config/ttymap/plugin/` (auto-discovered plugins) or
`~/.config/ttymap/lua/` (`require`'d shared libs).

## File locations

| Path | Content |
|------|---------|
| `~/.config/ttymap/init.lua` | Configuration |
| `~/.config/ttymap/plugin/` | User Lua plugins (`*.lua` or `<name>/init.lua`) |
| `~/.config/ttymap/lua/` | User shared libs / overrides for bundled `lua/` scripts |
| `~/.local/share/ttymap/plugin/` | Bundled Lua plugins (placed by `make install`) |
| `~/.local/share/ttymap/lua/ttymap/` | Bundled shared libs (`fmt`, `sidebar`) |
| `~/.local/share/ttymap/exports/` | Frame exports (palette → "Export current frame") |
| `~/.cache/ttymap/` | Disk tile cache |
| `~/.local/state/ttymap/ttymap.log` | Log file (auto-rotated at 1 MB; only when `--log` is passed) |
