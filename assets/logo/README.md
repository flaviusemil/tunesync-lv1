# TuneSync logo

**Idea.** A tuning fork whose stem is also the stem of a single note: one note comes in at the bottom and the fork's two matched tines ring in the same key, which is the "one MIDI note retunes every Tune instance" story in one glyph. The wordmark is built from the same U-bend (u, n, y) and one circle (e, c).

**Colours.** Tile `#1f2228` (panel), fork and "Sync" `#3fb37f` (accent), note head and "Tune" `#e8eaed`. On light pages "Tune" is `#16181c` and "Sync" is `#2b7d59`. Amber is not used.

**Files.**
- `tunesync-mark.svg`: the square symbol. It carries its own dark tile, so the one file works on dark and light backgrounds. Use it at 64 px and up.
- `tunesync-mark-16.svg`, `tunesync-mark-24.svg`, `tunesync-mark-32.svg`: pixel-fitted redraws for small sizes (heavier strokes on whole pixels, tighter margins). 48 px is the 24 px drawing at 2x.
- `tunesync-logo.svg` / `tunesync-logo-light.svg`: mark + wordmark for dark / light backgrounds, with 12 units of clear space built in.
- `tunesync.ico`: Windows icon holding 16, 24, 32, 48, 64, 128 and 256 px, each taken from the matching drawing. `png/` holds the review renders.
- `render.sh`: rebuilds `png/` and `tunesync.ico` from the SVGs (needs `rsvg-convert` and Python 3 with Pillow).

**Fonts.** None. The wordmark letters are original outlines drawn for this logo, so there is no font licence to track.
