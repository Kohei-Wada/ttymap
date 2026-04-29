# Lua bridge surface audit — 5 plugins

> Working doc for sizing the mlua bridge. Removable after the bridge ships.
> Date: 2026-04-29. Verified against tree at commit `2e1c6b7` by spot-grep.

Scope: every external API call made by the 5 plugins targeted for Lua migration:
`aircraft`, `iss`, `quake`, `wiki`, `here`.

Anything called by them is **must-expose** to Lua. Anything *not* called is candidate for v2 / cut. Duplications between APIs are flagged in §9.

---

## 1. `MapApi` (world-space draw primitives)

| Method | aircraft | iss | quake | wiki | here | n |
|---|---|---|---|---|---|---|
| `point(ll, ch, color)` | ✓ | ✓ | ✓ | ✓ | — | 4 |
| `accent_color()` | ✓ | — | ✓ | ✓ | — | 3 |
| `accent_alt_color()` | ✓ | ✓ | ✓ | ✓ | — | 4 |
| `label(ll, text, color)` | — | ✓ | — | — | — | 1 |

`MapApi` methods used by **none** of the 5 plugins (i.e. cuttable from v1 Lua bridge):
`center`, `zoom`, `area_width`, `cursor`, `cursor_ll`, `muted_color`, `point_styled`, `line`, `text_anchored`, `polyline` (any subset that exists).

Call sites:
- `aircraft/component.rs:85,86,95`
- `iss/component.rs:62,63,64`
- `quake/component.rs:37,38,45`
- `wiki/component.rs:134,135,142`

---

## 2. `Window` (event-handler side effects)

| Method | aircraft | iss | quake | wiki | here | n |
|---|---|---|---|---|---|---|
| `emit(AppMsg)` | ✓ | ✓ | — | ✓ | — | 3 |
| `ignore()` | ✓ | ✓ | — | ✓ | — | 3 |
| `ctx()` | ✓ | — | — | ✓ | — | 2 |
| `close()` | — | — | — | ✓ | — | 1 |
| `open(c)` / `toggle(c)` | — | — | — | — | — | 0 |

`here` doesn't call `Window::*` because it's a `Task`, not a `Component` — emits via `Task::poll() -> Vec<AppMsg>` (here/task.rs:22).

Call sites: `aircraft/component.rs:39,58,71,111`, `iss/component.rs:39,48`, `wiki/component.rs:34,40,57,85,98,114,125`.

---

## 3. `RenderWindow` (panel-space draw)

| Method | aircraft | iss | quake | wiki | here | n |
|---|---|---|---|---|---|---|
| `area() -> Rect` | ✓ | ✓ | — | ✓ | — | 3 |
| `style(StyleKind) -> TextStyle` | ✓ | ✓ | — | ✓ | — | 3 |
| `clear(rect)` | — | ✓ | — | ✓ | — | 2 |
| `paragraph(p, rect)` | indirect via `ListPanel` | ✓ | — | ✓ | — | 2 direct + 1 indirect |
| `panel(rect, title)` / `table(...)` / `ctx()` | — | — | — | — | — | 0 |

`quake` has no panel (paint_on_map only — markers, no UI panel).
`here` is headless — no `Component`, no render path.

aircraft routes its panel through `ListPanel { ... }.render(area, win)` which calls `win.paragraph` internally; from the plugin's POV `paragraph` is reached, but the descriptor is built by `ListPanel`, not by aircraft directly.

---

## 4. Widget descriptors (`src/widget/*`)

| Type | aircraft | iss | quake | wiki | here | n | notes |
|---|---|---|---|---|---|---|---|
| `Paragraph { lines, style, framed_title }` | indirect | ✓ | — | ✓ | — | 2+1 | aircraft via ListPanel |
| `Line::from_span` / `from_spans` | indirect | ✓ | — | ✓ | — | 2+1 | same |
| `Span::raw` / `Span::styled` | indirect | ✓ | — | ✓ | — | 2+1 | same |
| `Rect` | ✓ | ✓ | — | ✓ | — | 3 | layout outputs |
| `StyleKind::Body` | ✓ | ✓ | — | ✓ | — | 3 |
| `StyleKind::Muted` | — | ✓ | — | ✓ | — | 2 |
| `StyleKind::Selected` | ✓ | — | — | — | — | 1 |
| `StyleKind::Accent` | — | — | — | ✓ | — | 1 |
| `StyleKind::Highlight` | — | — | — | ✓ | — | 1 |
| `StyleKind::MutedFg` | — | — | — | ✓ | — | 1 |
| `Table` / `Cell` / `Row` / `ListItem` / `TableSel` / `List` | — | — | — | — | — | 0 |
| `Size` | — | — | — | — | — | 0 (used internally by widget impls) |

All 6 active `StyleKind` variants are used. Don't trim.

Cuttable from v1: `Table` family (rows/cells/sel), `widget::List` + `ListItem` (the descriptor `widget::List` is currently unused after the search migration removed `RenderWindow::list`).

---

## 5. `plugin_api::*` helpers

| Helper | aircraft | iss | quake | wiki | here | n | what it is |
|---|---|---|---|---|---|---|---|
| `PolledFeed<T>` | ✓ | ✓ | ✓ | ✓ | — | 4 | spawn fetch every N sec, poll for results |
| `LayoutConfig` + `PanelAnchor` | ✓ | ✓ | — | ✓ | — | 3 | user-overridable panel placement |
| `InitialJump` | — | ✓ | ✓ | — | — | 2 | one-shot auto-recenter on first data |
| `ListPanel` | ✓ | — | — | — | — | 1 | framed scrollable list chrome |
| `AsyncJob<T>` | — | — | — | — | ✓ | 1 | bare spawn-and-poll (no timer) |
| `Throttle` | — | — | — | — | — | 0 | only used internally by PolledFeed |
| `NominatimClient` | — | — | — | — | — | 0 | only `info` plugin + `search` provider use it |

Constructor calls:
- `PolledFeed::ready(Duration)` — aircraft (12s), iss (5s), quake (300s)
- `PolledFeed::with_cooldown(Duration)` — wiki (2s, delays first fetch)
- `AsyncJob::new()` + `AsyncJob::spawn(F)` + `AsyncJob::poll() -> Option<T>` — here

`PanelAnchor` variants in real use: `Left` (aircraft default), `TopLeft` (iss default), `Right` (wiki default). Other 4 (`TopRight`, `BottomLeft`, `BottomRight`, `Center`) are reachable via user config only.

---

## 6. `AppMsg` variants emitted

| Variant | aircraft | iss | quake | wiki | here | n |
|---|---|---|---|---|---|---|
| `Jump(LonLat)` | ✓ | ✓ | — | ✓ | ✓ | 4 |
| `Map(Action::*)` / `SetTheme` / `CursorMoved` / `CycleFocus` / `Resize` / `Quit` / `ExportFrame` / `Snapshot*` | — | — | — | — | — | 0 |

**Only `Jump` matters.** This is a huge simplification for the Lua bridge — the entire `AppMsg` enum can stay Rust-private; we expose a single `host.jump(lat, lon)` shorthand.

---

## 7. `Registrar::*` in `register()`

| Method | aircraft | iss | quake | wiki | here | n |
|---|---|---|---|---|---|---|
| `add_toggle(label, hint, factory)` | ✓ | ✓ | ✓ | ✓ | — | 4 |
| `bind(key, mods, factory)` | — | — | — | ✓ | — | 1 |
| `add_run(label, hint, action)` | — | — | — | — | ✓ | 1 |
| `add_task(box)` | — | — | — | — | ✓ | 1 |
| `add_spawn` / `add_overlay` / `add_activation` | — | — | — | — | — | 0 |

Wiki uses **both** `bind('i', …)` and `add_toggle("Toggle wiki", "i", …)` — the keybind opens it, the palette entry exists for discoverability and uses the same factory. This pattern (same component reachable from key + palette) might be worth a single `add_toggle_with_key('i', label, factory)` helper, since 3 of the 4 toggle-style plugins have an empty key hint and could opt in.

---

## 8. Aggregate — what to expose to Lua (v1 surface)

Sorted by usage. Bold = required for v1 (≥ 1 plugin uses it directly).

**MapApi (4 methods):** `point`, `label`, `accent_color`, `accent_alt_color`
**Window (4):** `emit`, `ignore`, `ctx`, `close`
**RenderWindow (4):** `area`, `style`, `paragraph`, `clear`
**Widgets (5 types):** `Paragraph`, `Line`, `Span`, `Rect`, `StyleKind` (all 6 variants)
**Helpers (5):** `PolledFeed`, `LayoutConfig`, `PanelAnchor`, `InitialJump`, `ListPanel`, `AsyncJob`
**AppMsg (1 variant):** `Jump`
**Registrar (4):** `add_toggle`, `bind`, `add_run`, `add_task`

Total ~27 distinct symbols. Adding `Component` trait method shape (handle_event / render / paint_on_map / poll / footer_hints / name) on top — that's the actual mlua bridge work.

---

## 9. Duplication / consolidation candidates

The user explicitly asked for this. Honest opinion on each:

### A. `Throttle` vs `PolledFeed` vs `AsyncJob`
Three abstractions in `plugin_api/`:
- `Throttle` — "has interval elapsed?" boolean check (`Throttle::check`)
- `AsyncJob<T>` — fire-and-forget thread + mpsc poll
- `PolledFeed<T>` = Throttle + AsyncJob fused

**Real usage**: `PolledFeed` covers 4 of 5 plugins. `AsyncJob` raw is used only by `here` (no interval — one-shot lookup). `Throttle` is internal to PolledFeed; no plugin uses it directly.

**Verdict**: Don't expose `Throttle` to Lua. Keep `AsyncJob` and `PolledFeed` — they're not duplicates, they're "with timer" and "without timer". Document the difference.

### B. `MapApi::point` vs `MapApi::point_styled`
`point(ll, ch, color)` and `point_styled(ll, ch, Style)` — only differ by `Color` vs `Style` arg. `point_styled` is `#[allow(dead_code)]`, no in-tree caller.

**Verdict**: Drop `point_styled` from v1 Lua bridge. Resurrect when a plugin actually needs background color or modifiers.

### C. `add_toggle` + `bind` redundancy in wiki
Wiki registers the same factory twice (key + palette entry). aircraft / iss / quake do `add_toggle` only with empty hint, which means they're palette-only (no key).

**Verdict**: Add a sugar `add_toggle_with_key(key, label, factory)` that wires both. Cuts 4 lines per plugin and makes the "key + palette" relationship a single declaration. Not strictly required for Lua but worth doing in Rust first.

### D. `AppMsg` exposure surface
4 of 5 plugins emit only `Jump`. The entire `AppMsg` vocabulary is overkill for Lua plugins.

**Verdict**: Don't expose `AppMsg` enum to Lua at all. Provide `host.jump(lat, lon)` as a typed primitive. If future plugins need other variants, add typed primitives one-by-one. Keeps the Lua API clean and the Rust enum private.

### E. `RenderWindow::paragraph` indirectness for aircraft
aircraft doesn't call `paragraph` directly — it builds `ListPanel { ... }` and the panel's own `render` method calls `win.paragraph`. From a Lua bridge perspective this means: if we expose `ListPanel`, we don't necessarily have to expose `Paragraph` for aircraft. But iss + wiki both build `Paragraph` by hand.

**Verdict**: Expose both. `ListPanel` is sugar for the common "framed scrollable list" case; raw `Paragraph` is for everyone else.

### F. `PanelAnchor` over-supply
7 variants exist, only 3 are used as defaults by plugins. The other 4 are only reachable via user config (which doesn't exist for the unimplemented plugins).

**Verdict**: Keep all 7. They're cheap (it's just an enum), users config them, and pruning would be visible to existing users.

### G. `StyleKind` semantics
6 active variants, with overlapping intent: `Body`, `Muted`, `MutedFg`, `Selected`, `Accent`, `Highlight`. `MutedFg` vs `Muted` is a particular question — what's the difference?

**Verdict**: Worth a closer read of `widget/style.rs` to confirm naming is intentional. Not Lua-specific — affects Rust API hygiene either way.

---

## 10. Single-use APIs (consider before exposing)

| API | Sole user | Verdict |
|---|---|---|
| `MapApi::label` | iss | **Keep.** Generic, only iss currently has on-map text but others might. |
| `Window::close` | wiki | **Keep.** Self-toggle is real semantic. |
| `ListPanel` | aircraft | **Keep.** Reusable sugar. |
| `AsyncJob<T>` | here | **Keep.** One-shot async without timer is a real shape. |
| `Registrar::bind` | wiki | **Keep.** Key-only registration is meaningful. |
| `Registrar::add_run` | here | **Keep.** Palette action without component. |
| `Registrar::add_task` | here | **Keep.** Headless background work. |

None of these are accidents. All 7 should be on the Lua surface.

---

## 11. Plugin shape patterns (informs Lua templates)

Across the 5 plugins, 4 distinct shapes:

1. **Fetch + render + select + jump** (aircraft, iss, wiki) — `PolledFeed`, panel + map markers, list selection, Enter→Jump.
2. **Fetch + render only, no selection** (quake) — markers only, no panel, no input. `InitialJump` for first-data centering.
3. **One-shot async with no UI** (here) — `Task` + `AsyncJob`, emits `Jump` from `poll()`.
4. **Modal hybrid** (wiki detail mode) — same component switches between list view and detail view; key handler is mode-dependent.

The Lua template library can ship 3 starter shapes (skipping #4 since wiki's complexity is the outlier). Cover ~80% of new-plugin demand with `fetch_render_select`, `fetch_render_only`, `oneshot_task`.

---

## 12. Implications for mlua bridge sizing

Given §8 (~27 symbols) and the simplifications in §9 (no `Throttle`, no `AppMsg` enum, no `point_styled`, no `Table`):

- **mlua dep + scaffolding**: ~50 LOC
- **`LuaComponent` adapter** (Component trait → Lua table dispatch): ~100 LOC
- **MapApi bridge** (4 methods + Color enum): ~40 LOC
- **Window bridge** (4 methods + `host.jump` shorthand): ~50 LOC
- **RenderWindow bridge** (4 methods): ~50 LOC
- **Widget descriptors as Lua tables** (Paragraph / Line / Span / Rect / StyleKind): ~80 LOC
- **Helpers** (PolledFeed / LayoutConfig / PanelAnchor / InitialJump / ListPanel / AsyncJob): ~120 LOC
- **Registrar bridge** (4 methods): ~40 LOC

**Total estimate: ~530 LOC.** Close to the original 560 LOC estimate from prior planning, *not* the 430 LOC optimistic case. Reason: I had hoped Lua would skip `Component` entirely by using only `PaletteProvider`, but the audit shows 4 of 5 plugins legitimately need full Component (panel + paint_on_map + key handling).

The savings vs the original number come from cutting `AppMsg` exposure (use `host.jump` typed call), `point_styled`, `Table`, `Throttle`, and `MapApi` methods that none of the 5 plugins use.

---

## 13. Open questions for the bridge design

1. Lua VM choice — Lua 5.4 vs LuaJIT. Default to mlua + Lua 5.4 + `vendored` (no system dep)?
2. Plugin loading — `include_str!`-bundled at compile time, or `~/.config/ttymap/plugins/*.lua` at runtime, or both?
3. Sandbox — disable `os.execute` / `io.*` for plugins, or trust them (personal-use scope)?
4. `Component::handle_event` semantics — should Lua's handler return an action enum (like the widget pattern), or call `win:emit/ignore/close` imperatively?
5. Errors in Lua — panic the host? log + ignore? deactivate the plugin?

These are bridge-design decisions, separate from the API surface itself.
