# ttymap travel demo —
#   palette → "Travel" → list → ↓×4 (Hiroshima + Setouchi, 5th)
#   → Enter (detail, pulled-back polyline preview)
#   → Enter (animated tour) → linger on post-tour overview.
# Run from repo root: `vhs vhs/travel.tape` → assets/travel.mp4

Output assets/travel.mp4

Set FontSize 16
Set Width 1920
Set Height 1080
Set Framerate 24
Set Theme "TokyoNight"
Set TypingSpeed 50ms
Set PlaybackSpeed 1.0

Type "ttymap"
Enter
Sleep 2s

# `:` opens the palette; "travel" filters to the Travel command.
Type ":"
Sleep 400ms
Type "travel"
Sleep 500ms
Enter
Sleep 1500ms

# List opens at row 1 (Japan · Golden Route). Navigate down to the
# 5th entry — Japan · Hiroshima + Setouchi.
Down
Sleep 250ms
Down
Sleep 250ms
Down
Sleep 250ms
Down
Sleep 800ms

# Enter opens detail — sidebar shows itinerary, map paints the whole
# polyline + all markers as a static pulled-back preview. Linger so
# the route shape reads.
Enter
Sleep 5s

# Enter again starts the tour: pre-fly → pre-dwell → 5 × (fly + 2 s
# dwell) → post-fly → post-dwell ≈ 30 s end-to-end. Extra trailing
# sleep keeps the post-overview (camera parked, full polyline +
# markers shown) on screen for a beat after the script ends.
Enter
Sleep 38s

# Back out: detail → list → close sidebar → quit ttymap.
Escape
Sleep 400ms
Escape
Sleep 400ms
Type "q"
Sleep 200ms
