-- ttymap.director — coroutine-based scheduler for choreographing
-- async actions over time. Wraps `ttymap.animation.fly_to` and a
-- frame-based wait timer behind a procedural API:
--
--   local director = require "ttymap.director"
--
--   director.run(function()
--       ttymap.notify("Starting tour")
--       director.fly(139.69, 35.69, 10)         -- yields until arrival
--       director.wait(120)                        -- yields 120 frames
--       for _, stop in ipairs(stops) do
--           director.fly(stop.lon, stop.lat, stop.zoom)
--           ttymap.notify(stop.note)
--           director.wait(120)
--       end
--   end, {
--       on_cancel = function() ttymap.notify("Cancelled") end,
--   })
--
-- Multiple scripts run in parallel — each call to `run` registers
-- a new coroutine. They're all driven by a single on_tick
-- subscription that advances waits, ticks tweens, and prunes dead
-- records.
--
-- Cancellation:
--   * `handle:cancel()` cancels one script. Pass `{ silent = true }`
--     to skip the on_cancel callback (use when the caller already
--     knows about the cancel and doesn't want a duplicate signal —
--     e.g. an explicit "stop tour" UI action).
--   * `director.cancel_all()` cancels every active script.
--   * The animation lib's tolerance check during a `director.fly`
--     (manual user pan / zoom) cancels the script via the lib's own
--     on_cancel — and fires the script's on_cancel.
-- Natural completion (the function returns) does NOT fire
-- on_cancel — it's reserved for "interrupted before finishing".
--
-- Primitives:
--   director.fly(lon, lat, zoom, opts?)   -- yields until arrival
--   director.wait(frames)                  -- yields N frames
--   director.tween(setter, frames)         -- yields, calls
--                                             setter(t in [0,1])
--                                             every frame for
--                                             `frames` frames
--   ttymap.notify(...)                     -- not a director
--                                             primitive — synchronous,
--                                             call directly
--
-- This is the kernel of "scriptable scenes" in ttymap. The travel
-- plugin's pre/stop/post tour, ping_simulation's per-ping loops,
-- a future demo plugin's auto-pan-zoom — all collapse to
-- procedural code.

local anim = require "ttymap.animation"

local M = {}
local actives = {}

-- Tolerances for "did the user touch the map during a wait"
-- detection. Same values the animation lib uses for its own
-- pre-emption check — loose enough to absorb the float round-trip
-- through `geo::normalize` + Mercator clamps, tight enough that a
-- single-cell keyboard pan trips them.
local TOL_LL   = 0.0005
local TOL_ZOOM = 0.001

-- ── Handle / cancellation ───────────────────────────────────────
local Handle = {}
Handle.__index = Handle

local function cancel_rec(rec, silent)
    if rec.dead then return end
    rec.dead = true
    rec.cancelled = true
    if not silent and rec.on_cancel then
        local ok, err = pcall(rec.on_cancel)
        if not ok then
            ttymap.log:warn("director: on_cancel raised: " .. tostring(err))
        end
    end
end

function Handle:cancel(opts)
    cancel_rec(self, opts and opts.silent)
end

function M.cancel_all()
    for _, rec in ipairs(actives) do
        cancel_rec(rec)
    end
end

-- ── Step + dispatch ─────────────────────────────────────────────
-- Resume rec's coroutine and act on whatever directive it yields.
-- Marks rec.dead if the script finished naturally or errored out.
local function step(rec)
    if rec.dead then return end
    local ok, directive = coroutine.resume(rec.co)
    if not ok then
        ttymap.log:warn("director: coroutine error: " .. tostring(directive))
        rec.dead = true
        return
    end
    if coroutine.status(rec.co) == "dead" then
        -- Natural completion. No on_cancel — rec.cancelled stays false.
        rec.dead = true
        return
    end
    rec.directive = directive
    if directive.kind == "fly" then
        local opts = directive.opts or {}
        anim.fly_to(directive.lon, directive.lat, directive.zoom, {
            frames    = opts.frames,
            on_done   = function()
                if not rec.dead then
                    rec.directive = nil
                    step(rec)
                end
            end,
            on_cancel = function() cancel_rec(rec) end,
        })
    elseif directive.kind == "wait" then
        rec.wait_left = directive.frames
        -- Captured on the first wait tick (deferred). The just-
        -- completed fly's on_done runs *inside* the animation
        -- on_tick, which means drain_ops hasn't yet applied its
        -- final FlyTo to MapState — reading `:center()` here would
        -- snapshot the pre-fly value and the next tick would falsely
        -- diverge. One-frame defer lets MapState settle.
        rec.wait_check = nil
    elseif directive.kind == "tween" then
        rec.tween_elapsed = 0
    end
end

-- ── Public ──────────────────────────────────────────────────────
function M.run(fn, opts)
    local rec = setmetatable({
        co        = coroutine.create(fn),
        dead      = false,
        cancelled = false,
        on_cancel = opts and opts.on_cancel,
    }, Handle)
    step(rec)
    if not rec.dead then
        table.insert(actives, rec)
    end
    return rec
end

function M.fly(lon, lat, zoom, opts)
    return coroutine.yield({
        kind = "fly",
        lon  = lon, lat = lat, zoom = zoom,
        opts = opts,
    })
end

function M.wait(frames)
    return coroutine.yield({ kind = "wait", frames = frames })
end

function M.tween(setter, frames)
    -- Guard frames=0 (would divide-by-zero in the per-tick driver
    -- at `tween_elapsed / d.frames`) and negatives. Clamp to 1 so
    -- the setter still fires with a final value rather than
    -- silently dropping the call.
    if frames < 1 then frames = 1 end
    return coroutine.yield({
        kind   = "tween",
        setter = setter,
        frames = frames,
    })
end

-- ── Driver ──────────────────────────────────────────────────────
-- Per-tick: prune dead records, decrement wait timers, advance
-- tween setters. "fly" directives don't need per-tick attention —
-- they advance via the animation lib's on_done callback wired up
-- in step().
ttymap.api.frame.on_tick(function()
    local i = 1
    while i <= #actives do
        local rec = actives[i]
        if rec.dead then
            table.remove(actives, i)
        else
            local d = rec.directive
            local advance = false
            local cancelled_in_tick = false
            if d then
                if d.kind == "wait" then
                    -- Detect manual user input during wait: any
                    -- divergence between the captured map state and
                    -- the live one means the user (or another
                    -- caller) moved the camera; bail the script.
                    -- First tick captures, subsequent ticks compare.
                    if not rec.wait_check then
                        local lon, lat = ttymap.map:center()
                        rec.wait_check = {
                            lon = lon, lat = lat,
                            zoom = ttymap.map:zoom(),
                        }
                    else
                        local lon, lat = ttymap.map:center()
                        local zoom = ttymap.map:zoom()
                        if math.abs(lon - rec.wait_check.lon) > TOL_LL
                            or math.abs(lat - rec.wait_check.lat) > TOL_LL
                            or math.abs(zoom - rec.wait_check.zoom) > TOL_ZOOM then
                            cancel_rec(rec)
                            cancelled_in_tick = true
                        end
                    end
                    if not cancelled_in_tick then
                        rec.wait_left = rec.wait_left - 1
                        if rec.wait_left <= 0 then advance = true end
                    end
                elseif d.kind == "tween" then
                    rec.tween_elapsed = rec.tween_elapsed + 1
                    local t = math.min(rec.tween_elapsed / d.frames, 1)
                    local ok, err = pcall(d.setter, t)
                    if not ok then
                        ttymap.log:warn("director: tween setter error: " .. tostring(err))
                        cancel_rec(rec)
                    elseif t >= 1 then
                        advance = true
                    end
                end
                -- "fly": waits on anim's on_done — no per-tick work.
            end
            if cancelled_in_tick or rec.dead then
                table.remove(actives, i)
            elseif advance then
                rec.directive = nil
                step(rec)
                if rec.dead then
                    table.remove(actives, i)
                else
                    i = i + 1
                end
            else
                i = i + 1
            end
        end
    end
end)

return M
