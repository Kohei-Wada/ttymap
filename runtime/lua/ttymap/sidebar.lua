-- ttymap.sidebar — boilerplate helpers for sidebar plugins.
--
-- Bundled plugins (aircraft, quake, wiki, satellite, …) repeat the
-- same three patterns:
--
-- 1. Cycle a `selected` index through 1..n on Up / Down / C-p / C-n.
-- 2. Recognise q / Esc as the close key.
-- 3. Wrap `w:close() + w = nil` in a local `close()` function so
--    the plugin's window-handle ref is reset along with the
--    compositor stack pop (otherwise re-toggling sees a stale `w`
--    and tries to close again).
--
-- This module collapses the three into helpers:
--   sidebar.up_pressed(key)         -> bool   (C-p OR Up)
--   sidebar.down_pressed(key)       -> bool   (C-n OR Down)
--   sidebar.is_close_key(key)       -> bool   (q OR Esc)
--   sidebar.cycle(idx, n, dir)      -> int    (1..n with wraparound)
--
-- Plugins still own their own `w` variable and decide what to do on
-- close (e.g. quake separates feed-enabled from panel-open). The
-- helpers normalise *key recognition* and *index arithmetic* without
-- imposing a lifecycle.

local M = {}

--- True for "scroll up one row" or "select previous item": Up arrow
--- (no Ctrl) or C-p.
function M.up_pressed(key)
    if key.code == "Up" and not key.ctrl then return true end
    if key.code == "Char" and key.char == "p" and key.ctrl then return true end
    return false
end

--- True for "scroll down one row" or "select next item": Down arrow
--- (no Ctrl) or C-n.
function M.down_pressed(key)
    if key.code == "Down" and not key.ctrl then return true end
    if key.code == "Char" and key.char == "n" and key.ctrl then return true end
    return false
end

--- True for the close key: q (no Ctrl) or Esc.
function M.is_close_key(key)
    if key.code == "Esc" then return true end
    if key.code == "Char" and key.char == "q" and not key.ctrl then return true end
    return false
end

--- Move 1..n with wraparound. `dir` is +1 (next) or -1 (prev).
--- Returns the new index. When `n == 0` returns 1 (callers should
--- guard against an empty list anyway).
function M.cycle(idx, n, dir)
    if n <= 0 then return 1 end
    if dir > 0 then
        return idx < n and idx + 1 or 1
    else
        return idx > 1 and idx - 1 or n
    end
end

return M
