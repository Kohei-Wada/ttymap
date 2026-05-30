-- traceroute — animated traceroute viz over the globe.
--
-- `:` → "Traceroute" (or `/` style palette opens directly via the
-- registered palette command). User types a hostname, Enter spawns
-- `traceroute -n` in the background. Each tick the plugin re-reads
-- the output file, enqueues an `http:fetch_cached` lookup for every
-- new public hop, and animates a polyline that grows hop-by-hop as
-- coordinates resolve.
--
-- Reuses the polyline overlay infrastructure (same as
-- `ping_simulation`): per frame, the plugin pushes the current chain
-- via `map:polyline` + `map:point`. The line stays visible across
-- frames because we keep redrawing it; starting a new traceroute
-- (or restarting ttymap) is the only way to clear it.
--
-- Endpoints / cadence / colours come from `ttymap.traceroute`.
-- Override via `require("ttymap.traceroute").<field> = ...` in
-- init.lua.

local config = require("ttymap.traceroute")
local sidebar = require("ttymap.sidebar")
local anim = require("ttymap.animation")

------------------------------------------------------------------
-- Helpers
------------------------------------------------------------------

-- Strict allowlist — chars valid in IPv4 / IPv6 literals + DNS labels.
-- Keeps the shell command template injection-safe: even when the
-- template substitutes the host directly, none of these chars carry
-- shell meaning.
local HOST_PATTERN = "^[%w%.%-_:]+$"

local function valid_host(s)
    return type(s) == "string"
       and #s > 0 and #s < 256
       and s:match(HOST_PATTERN) ~= nil
end

-- ip-api.com returns `{"status":"fail","message":"private range"}` for
-- RFC1918 / loopback / link-local addresses, so we could just let the
-- API tell us. Filtering locally saves a round-trip per private hop
-- (typical home network: 1-3 hops).
local function is_private_ipv4(ip)
    local a, b = ip:match("^(%d+)%.(%d+)%.")
    if not a then return false end
    a, b = tonumber(a), tonumber(b)
    if a == 10 or a == 127 then return true end
    if a == 192 and b == 168 then return true end
    if a == 169 and b == 254 then return true end
    if a == 172 and b >= 16 and b <= 31 then return true end
    return false
end

-- Same trick as `ping_simulation.interp_lon`: pick the shortest arc
-- around the antimeridian so e.g. Tokyo→NY traces over the Pacific
-- instead of via Eurasia/Atlantic.
local function interp_lon(src_lon, dst_lon, t)
    local dlon = dst_lon - src_lon
    if dlon > 180 then
        dlon = dlon - 360
    elseif dlon < -180 then
        dlon = dlon + 360
    end
    local lon = src_lon + dlon * t
    if lon > 180 then
        lon = lon - 360
    elseif lon < -180 then
        lon = lon + 360
    end
    return lon
end

-- Per-VM unique tmpfile name. Seeded at module load with a microsecond
-- fraction so two ttymap processes started in the same second don't
-- collide on the same /tmp path.
math.randomseed(os.time() + math.floor((os.clock() % 1) * 1e9))

local function make_tmpfile()
    return string.format("/tmp/ttymap-tr-%d-%d.log",
                         os.time(), math.random(100000, 999999))
end

------------------------------------------------------------------
-- Session state
------------------------------------------------------------------

-- Active traceroute, or nil. A session lives until the next traceroute
-- is started (or the VM exits). After cmd_done && all hops settled,
-- the polyline stays visible and on_tick keeps redrawing it cheaply.
--
-- `card_handle` (sidebar panel) is intentionally separate from
-- `session.visible` so hiding the display doesn't stop the bg cmd /
-- geoip work — the trace keeps progressing in the background and
-- per-hop notifies still fire even while invisible.
local session = nil
local tick_handle = nil
local card_handle = nil

local function abandon_session(s)
    if not s then return end
    -- Cancel in-flight geoip jobs so their workers stop targeting our
    -- (now-discarded) state. The HTTP request itself runs to
    -- completion in the background — `cancel()` only suppresses the
    -- result.
    for _, h in ipairs(s.hops) do
        if h.job then h.job:cancel() end
    end
    -- Best-effort tmpfile cleanup. The bg traceroute may still be
    -- writing to it when this fires; rm + ENOENT is harmless and the
    -- file is in /tmp anyway.
    if s.tmpfile then os.remove(s.tmpfile) end
end

------------------------------------------------------------------
-- Output reader — re-read the entire tmpfile each tick. Output is
-- small (one short line per hop, ≤30 hops), so parsing it fresh is
-- cheaper than tracking a partial-line read offset. We dedupe by
-- hop number so already-enqueued hops aren't re-fired.
------------------------------------------------------------------

-- Returns true once the bg shell wrote the sentinel.
local function poll_file(s)
    local f = io.open(s.tmpfile, "rb")
    if not f then return false end
    local content = f:read("a") or ""
    f:close()

    local seen_done = false
    for line in content:gmatch("[^\n]+") do
        if line == "__DONE__" then
            seen_done = true
        else
            -- " 4  93.184.216.34  20.123 ms"  → ("4", "93.184.216.34")
            -- " 3  *"                          → ("3", "*")
            local hop_str, token = line:match("^%s*(%d+)%s+(%S+)")
            if hop_str then
                local hop_num = tonumber(hop_str)
                if not s.hop_index[hop_num] then
                    local h = { num = hop_num }
                    if token == "*" then
                        h.status = "timeout"
                    elseif is_private_ipv4(token) then
                        h.status = "private"
                        h.ip = token
                    else
                        h.status = "pending"
                        h.ip = token
                        h.job = ttymap.http:fetch_cached(
                            config.geoip_url(token),
                            config.geoip_ttl_s)
                    end
                    s.hop_index[hop_num] = h
                    table.insert(s.hops, h)
                end
            end
        end
    end
    -- traceroute writes lines in hop-num order, but we sort defensively
    -- so the chain is always sorted regardless of read interleaving.
    table.sort(s.hops, function(a, b) return a.num < b.num end)
    return seen_done
end

-- Drain settled geoip jobs, transitioning "pending" hops to "resolved"
-- (with lon/lat) or "geoip_failed" (lookup returned but unusable). On
-- successful resolve, fire a notify so the user can correlate each
-- hop with where on the globe it landed without reading the map.
local function drain_jobs(s)
    for _, h in ipairs(s.hops) do
        if h.status == "pending" and h.job then
            local body = h.job:try_take()
            if body then
                local lon, lat = config.geoip_parse(body)
                if lon and lat then
                    h.lon, h.lat = lon, lat
                    h.status = "resolved"
                    ttymap.notify(string.format(
                        "hop %d  %s  (%.2f, %.2f)",
                        h.num, h.ip, lat, lon))
                else
                    h.status = "geoip_failed"
                end
                h.job = nil
            end
        end
    end
end

------------------------------------------------------------------
-- Animation. The "chain" is the ordered list of resolved hops (the
-- only ones with coords). The tip walks segment-by-segment along it,
-- pausing at the last available hop until more resolve.
------------------------------------------------------------------

local function compute_chain(s)
    local chain = {}
    for _, h in ipairs(s.hops) do
        if h.status == "resolved" then
            table.insert(chain, h)
        end
    end
    return chain
end

-- Build the panel's row list, collapsing runs of consecutive
-- `timeout` hops into one summary row. Internet backbone routers
-- routinely refuse / rate-limit ICMP TTL exceeded, so a typical
-- trace has 5–10× more `*` rows than meaningful hops; collapsing
-- keeps the panel readable without dropping the information.
--
-- Each row is one of:
--   { kind = "hop",      hop = <hop table> }
--   { kind = "timeouts", count = N, first = a, last = b }
local function compute_display_rows(s)
    local rows = {}
    local i = 1
    while i <= #s.hops do
        local h = s.hops[i]
        if h.status == "timeout" then
            local first = h.num
            local last  = h.num
            local count = 1
            while i + count <= #s.hops
                and s.hops[i + count].status == "timeout" do
                last  = s.hops[i + count].num
                count = count + 1
            end
            table.insert(rows, {
                kind = "timeouts", count = count,
                first = first, last = last,
            })
            i = i + count
        else
            table.insert(rows, { kind = "hop", hop = h })
            i = i + 1
        end
    end
    return rows
end

-- tip_idx = i + tip_progress in [0, 1] means "currently at position
-- chain[i] → interpolating toward chain[i+1] at progress p". We clamp
-- to (1, 0) on entry — the very first frame draws just the marker
-- with no segment yet.
local function advance_tip(s, n_chain)
    if n_chain <= 1 then return end
    if s.tip_idx >= n_chain then return end  -- already at last hop
    s.tip_progress = s.tip_progress + (1.0 / config.seg_frames)
    while s.tip_progress >= 1.0 and s.tip_idx < n_chain do
        s.tip_progress = s.tip_progress - 1.0
        s.tip_idx = s.tip_idx + 1
    end
    if s.tip_idx >= n_chain then
        s.tip_progress = 0.0
    end
end

-- `hop_color` accepts either a static xterm-256 index or a function
-- of the hop number; normalise here so the draw loop doesn't care.
local function color_for(hop_num)
    local c = config.hop_color
    if type(c) == "function" then return c(hop_num) end
    return c
end

local function draw(map, s, chain)
    -- Markers + hop numbers at every resolved hop, regardless of where
    -- the tip currently is. The numbers help readers correlate the
    -- on-screen viz with the underlying traceroute output.
    for _, h in ipairs(chain) do
        local c = color_for(h.num)
        map:point(h.lon, h.lat, config.marker_glyph, c)
        map:label(h.lon, h.lat, tostring(h.num), c)
    end

    -- Completed segments — one polyline per pair so each segment can
    -- carry the destination hop's colour. (A single multi-point
    -- polyline would force one colour for the whole chain.)
    for i = 1, s.tip_idx - 1 do
        local from = chain[i]
        local to   = chain[i + 1]
        if from and to then
            map:polyline({ { from.lon, from.lat }, { to.lon, to.lat } },
                         color_for(to.num))
        end
    end

    -- Growing tip segment — coloured by the hop it's heading toward,
    -- so the line fades into the destination marker's hue as it
    -- arrives.
    if s.tip_idx < #chain and s.tip_progress > 0 then
        local from = chain[s.tip_idx]
        local to   = chain[s.tip_idx + 1]
        local lon  = interp_lon(from.lon, to.lon, s.tip_progress)
        local lat  = from.lat + (to.lat - from.lat) * s.tip_progress
        map:polyline({ { from.lon, from.lat }, { lon, lat } },
                     color_for(to.num))
    end
end

------------------------------------------------------------------
-- Tick driver
------------------------------------------------------------------

local function any_pending(s)
    for _, h in ipairs(s.hops) do
        if h.status == "pending" then return true end
    end
    return false
end

local function on_tick(map)
    if not session then return end

    if not session.cmd_done then
        if poll_file(session) then session.cmd_done = true end
    end
    drain_jobs(session)
    local chain = compute_chain(session)
    session.display_rows = compute_display_rows(session)
    -- Animation advances regardless of visibility so the line picks up
    -- where it would have been when the user toggles the display back
    -- on (rather than freezing while hidden then jumping forward).
    advance_tip(session, #chain)
    if session.visible then
        draw(map, session, chain)
    end

    -- One-shot completion notify: cmd exited, all geoip jobs settled,
    -- tip has caught up to the last resolved hop.
    if session.cmd_done
        and not session.notified_done
        and not any_pending(session)
        and (#chain == 0 or session.tip_idx >= #chain) then
        session.notified_done = true
        ttymap.notify(string.format(
            "traceroute %s: %d hop%s resolved",
            session.host, #chain, #chain == 1 and "" or "s"))
    end
end

------------------------------------------------------------------
-- Sidebar panel — list of hops with status / IP / coords. The same
-- live `session` table backs both the map overlay and this card, so
-- every per-tick mutation surfaces here automatically; the bridge
-- re-asks `items()` / `selected()` each redraw.
------------------------------------------------------------------

-- Status → sidebar style. Resolved hops carry the accent so the
-- meaningful rows stand out from waiting / failed ones.
local function status_style(status)
    if status == "resolved" then return "accent" end
    if status == "pending"  then return "muted" end
    return "muted_fg"
end

-- Render fallback — only used when build_items returns []. The
-- card bridge picks this up automatically when the items list is
-- empty.
local function build_lines()
    if not session then
        return {
            { { text = "(no trace)",                   style = "muted" } },
            { { text = "Run :Traceroute to host",       style = "muted" } },
        }
    end
    return {
        { { text = "Tracing " .. session.host,         style = "accent" } },
        { { text = "(waiting for first hop…)",          style = "muted" } },
    }
end

local function build_items()
    if not session or not session.display_rows then return {} end
    local items = {}
    for _, row in ipairs(session.display_rows) do
        if row.kind == "hop" then
            local h = row.hop
            local primary, secondary
            if h.status == "resolved" then
                primary   = string.format("%2d  %s", h.num, h.ip)
                secondary = string.format("    %.2f, %.2f", h.lat, h.lon)
            elseif h.status == "pending" then
                primary   = string.format("%2d  %s", h.num, h.ip)
                secondary = "    resolving…"
            elseif h.status == "private" then
                primary   = string.format("%2d  %s", h.num, h.ip)
                secondary = "    (private)"
            else  -- geoip_failed / unknown
                primary   = string.format("%2d  %s", h.num, h.ip or "?")
                secondary = "    geoip failed"
            end
            table.insert(items, {
                { { text = primary,   style = status_style(h.status) } },
                { { text = secondary, style = "muted" } },
            })
        else  -- timeouts (collapsed run)
            local label
            if row.count == 1 then
                label = string.format("%2d  *  timeout", row.first)
            else
                label = string.format("%2d–%2d  %d× *  timeout",
                                      row.first, row.last, row.count)
            end
            table.insert(items, {
                { { text = label, style = "muted_fg" } },
            })
        end
    end
    return items
end

local function close_card()
    if card_handle then
        card_handle:close()
        card_handle = nil
    end
end

local function open_card()
    if card_handle then return end
    card_handle = ttymap.api.card.open({
        name = "traceroute",
        footer_hints = {
            { key = "C-n/C-p", label = "select" },
            { key = "Enter",   label = "fly to" },
            { key = "q / Esc", label = "close" },
        },
        render = build_lines,
        items  = build_items,
        selected = function()
            if not session or not session.display_rows then return 1 end
            local n = #session.display_rows
            if session.selected > n then
                session.selected = math.max(1, n)
            end
            return session.selected
        end,
        handle_key = function(key)
            local rows = session and session.display_rows or {}
            local n = #rows
            if sidebar.up_pressed(key) and session then
                session.selected = sidebar.cycle(session.selected, n, -1)
                return nil
            end
            if sidebar.down_pressed(key) and session then
                session.selected = sidebar.cycle(session.selected, n, 1)
                return nil
            end
            if key.code == "Enter" and session then
                local row = rows[session.selected]
                if row and row.kind == "hop"
                    and row.hop.status == "resolved" then
                    anim.fly_to(row.hop.lon, row.hop.lat)
                end
                return nil
            end
            if sidebar.is_close_key(key) then
                -- Close key on the panel hides the whole display so
                -- "Toggle traceroute display" is a single coherent
                -- on/off — overlay + panel are one user-facing
                -- concept.
                if session then session.visible = false end
                close_card()
                return nil
            end
            return { ignore = true }
        end,
    })
end

-- Sync card to session.visible. Bg work (cmd / geoip / notifies)
-- keeps running regardless.
local function update_display()
    if session and session.visible then
        if not card_handle then open_card() end
    else
        close_card()
    end
end

local function toggle_display()
    if not session then
        ttymap.notify("traceroute: no active trace yet",
                      { level = "warn" })
        return
    end
    session.visible = not session.visible
    update_display()
end

------------------------------------------------------------------
-- Session lifecycle
------------------------------------------------------------------

local function start_session(host)
    abandon_session(session)
    session = {
        host         = host,
        tmpfile      = make_tmpfile(),
        hops         = {},
        hop_index    = {},
        tip_idx      = 1,
        tip_progress = 0.0,
        cmd_done     = false,
        visible      = true,
        selected     = 1,
        display_rows = {},
    }

    -- (cmd > tmpfile 2>&1; echo __DONE__ >> tmpfile) &
    --
    -- The trailing `&` daemonizes the whole subshell so `os.execute`
    -- returns immediately. The sentinel lets the polling loop
    -- distinguish "no new lines yet" from "traceroute exited".
    local cmd = string.format(config.command, host)
    local shell = string.format(
        "(%s > %s 2>&1; echo __DONE__ >> %s) &",
        cmd, session.tmpfile, session.tmpfile)
    os.execute(shell)

    if not tick_handle then
        tick_handle = ttymap.api.frame.on_tick(on_tick)
    end
    update_display()
    ttymap.notify("traceroute → " .. host)
end

------------------------------------------------------------------
-- Palette UI — host prompt
------------------------------------------------------------------

local prompt_state = { query = "" }

local function open_prompt()
    prompt_state.query = ""
    ttymap.api.palette.open({
        prompt      = "host> ",
        submit_mode = "on_enter",

        filter = function(query)
            prompt_state.query = query:match("^%s*(.-)%s*$") or ""
        end,

        items = function()
            if prompt_state.query == "" then return {} end
            local label = "Trace " .. prompt_state.query
            if not valid_host(prompt_state.query) then
                return { { label = label, hint = "invalid host" } }
            end
            return { { label = label, hint = "Enter to run" } }
        end,

        execute = function(_idx)
            local host = prompt_state.query
            if valid_host(host) then
                start_session(host)
            elseif host ~= "" then
                ttymap.notify(
                    "traceroute: invalid host \"" .. host .. "\"",
                    { level = "warn" })
            end
        end,

        is_loading = function() return false end,
    })
end

ttymap.register_keybind("r", open_prompt)

ttymap.register_palette_command({
    label  = "Traceroute to host",
    hint   = "r",
    invoke = open_prompt,
})

ttymap.register_palette_command({
    label  = "Toggle traceroute display",
    invoke = toggle_display,
})
