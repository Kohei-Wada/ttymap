# ttymap nav demo —
#   help (?) → world → Japan zoom → London search → zoom out → pan back east.
# Run from repo root: `vhs vhs/nav.tape` → assets/nav.gif + assets/nav.mp4

Output assets/nav.mp4

Set FontSize 16
Set Width 1920
Set Height 1080
Set Framerate 24
Set Theme "TokyoNight"
Set TypingSpeed 50ms
Set PlaybackSpeed 1.0

# Start at a world-ish zoom centered on Japan.
Type "ttymap --lat 35.68 --lon 139.76 --zoom 2"
Enter
Sleep 1500ms

# `?` opens the help cheatsheet — leave it open through the rest of
# the demo as a viewer-side reference.
Type "?"
Sleep 2s

# Zoom in deep with `a` (≈50 presses → zoom ~13).
Type "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
Sleep 1500ms

# `/` opens the palette pre-loaded with the search provider.
# search uses submit_mode = "on_enter": first Enter triggers the
# Nominatim fetch (typing buffers silently), wait for the response,
# then a second Enter selects the top result and jumps.
Type "/"
Sleep 400ms
Type "London"
Sleep 400ms
Enter
Sleep 2500ms
Enter
Sleep 3500ms

# Zoom out fully back to a world view.
Type "zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz"
Sleep 1500ms

# `w` is fast pan east — travel back from London all the way to Japan.
Type "wwwwwwwwwwwwwwwwwwwwwwwwwwwwwwwwwwww"
Sleep 2500ms

Type "q"
Sleep 200ms
