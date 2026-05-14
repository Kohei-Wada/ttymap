-- ttymap.location — shared user-location cache.
--
-- Resolves "where is the user" (IP geoip) once and serves the answer
-- to every plugin that asks. Three layers: in-memory (this session),
-- ttymap.storage (across sessions, TTL-bounded), and HTTP fetch (the
-- canonical source). Plugins that previously rolled their own geoip
-- fetch state machine can replace it with a single `loc.get(cb)`.
--
-- Usage:
--   local loc = require "ttymap.location"
--
--   -- Async getter — cb fires sync on cache hit, async on network.
--   -- Concurrent calls coalesce onto a single in-flight fetch.
--   -- On failure cb fires with (nil, nil, "error").
--   loc.get(function(lat, lon, source)
--       if lat then ... else ... end
--   end)
--
--   -- Force refresh — skip TTL, always hit the network.
--   loc.refresh(function(lat, lon, source) end)
--
--   -- Sync read — returns nil, nil on miss or stale. Never fetches.
--   local lat, lon = loc.cached()
--
-- Config (mutate via `require("ttymap.location").<field> = ...`):
--   ttl_seconds  TTL for cache freshness (default 24h)
--
-- Endpoint config is shared with the `here` plugin — override via
-- `require("ttymap.here").endpoint = "..."`.

local here = require "ttymap.here"

local M = {
    ttl_seconds = 24 * 60 * 60,
}

local STORE_NS  = "location"
local STORE_KEY = "here"

local store_handle = nil   -- resolved lazily; ttymap.storage may not
                           -- exist when the module is loaded (no XDG
                           -- data dir → see api/storage.rs `new()`).

local _mem         = nil   -- { lat, lon, ts } or nil
local _job         = nil   -- in-flight ttymap.http job, if any
local _waiters     = {}    -- callbacks awaiting the current job
local _tick_handle = nil   -- on_tick subscription while job in flight

local function now()
    return os.time()
end

local function fresh(entry)
    if not entry then return false end
    return (now() - (entry.ts or 0)) < M.ttl_seconds
end

local function store()
    if not store_handle then
        if not (ttymap and ttymap.storage) then
            return nil
        end
        store_handle = ttymap.storage:open(STORE_NS)
    end
    return store_handle
end

local function load_from_storage()
    local s = store()
    if not s then return nil end
    local hit = s:get(STORE_KEY, nil)
    if hit
        and type(hit.lat) == "number"
        and type(hit.lon) == "number"
        and type(hit.ts)  == "number" then
        return hit
    end
    return nil
end

local function save_to_storage(entry)
    local s = store()
    if not s then return end
    s:set(STORE_KEY, entry)
end

-- Drain `_waiters` exactly once, swapping the buffer first so that a
-- callback re-entering via `loc.get` enqueues into a fresh list.
local function notify_waiters(lat, lon, source)
    local pending = _waiters
    _waiters = {}
    for _, cb in ipairs(pending) do
        cb(lat, lon, source)
    end
end

local function clear_job()
    _job = nil
    if _tick_handle then
        _tick_handle:remove()
        _tick_handle = nil
    end
end

local function drain()
    if not _job then return end
    local body = _job:try_take()
    if not body then return end

    clear_job()

    local parsed = ttymap.json:parse(body)
    if parsed
        and type(parsed.latitude)  == "number"
        and type(parsed.longitude) == "number" then
        local entry = {
            lat = parsed.latitude,
            lon = parsed.longitude,
            ts  = now(),
        }
        _mem = entry
        save_to_storage(entry)
        notify_waiters(entry.lat, entry.lon, "network")
    else
        ttymap.notify("location: geoip response missing lat/lon",
                      { level = "warn" })
        notify_waiters(nil, nil, "error")
    end
end

-- Kick off the HTTP fetch + subscribe to on_tick. Idempotent — extra
-- callers piggyback on the in-flight job via `_waiters`.
local function start_fetch()
    if _job then return end
    _job = ttymap.http:fetch(here.endpoint)
    _tick_handle = ttymap.api.frame.on_tick(drain)
end

function M.get(cb)
    if type(cb) ~= "function" then
        error("ttymap.location.get: callback must be a function", 2)
    end

    if fresh(_mem) then
        cb(_mem.lat, _mem.lon, "memory")
        return
    end

    local disk = load_from_storage()
    if fresh(disk) then
        _mem = disk
        cb(disk.lat, disk.lon, "storage")
        return
    end

    _waiters[#_waiters + 1] = cb
    start_fetch()
end

function M.refresh(cb)
    if type(cb) ~= "function" then
        error("ttymap.location.refresh: callback must be a function", 2)
    end
    _waiters[#_waiters + 1] = cb
    start_fetch()
end

function M.cached()
    if fresh(_mem) then
        return _mem.lat, _mem.lon
    end
    local disk = load_from_storage()
    if fresh(disk) then
        _mem = disk
        return disk.lat, disk.lon
    end
    return nil, nil
end

return M
