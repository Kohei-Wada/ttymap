# ttymap ping_simulation demo —
#   world view → palette → toggle "ping simulation" → watch animated arcs.
# Run from repo root: `vhs vhs/ping_simulation.tape` → assets/ping_simulation.mp4

Output assets/ping_simulation.mp4

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

# `gg` jumps to the world view so all five ping arcs fit on screen.
Type "gg"
Sleep 1500ms

# `:` opens the palette; "ping" filters to "Toggle ping simulation";
# Enter invokes it.
Type ":"
Sleep 400ms
Type "ping"
Sleep 500ms
Enter
Sleep 500ms

# Each ping cycle is out(45) + return(45) + pause(20) ≈ 110 frames;
# the five pings are staggered (offset 0/18/36/54/72), so the visual
# rhythm fully settles after the first ~1.5 s. ~12 s captures several
# full cycles across all five arcs.
Sleep 12s

Type "q"
Sleep 200ms
