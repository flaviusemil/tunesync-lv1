#!/bin/sh
# Regenerates png/ and tunesync.ico from the SVGs. Needs rsvg-convert and Python 3 + Pillow.
set -eu
cd "$(dirname "$0")"
mkdir -p png
r() { rsvg-convert -w "$2" -h "$2" "$1" -o "png/tunesync-mark-$2.png"; }
r tunesync-mark-16.svg 16
r tunesync-mark-24.svg 24
r tunesync-mark-32.svg 32
r tunesync-mark-24.svg 48   # the 24 px drawing at exactly 2x
for s in 64 128 256 512; do r tunesync-mark.svg "$s"; done
rsvg-convert -w 1200 -b '#16181c' tunesync-logo.svg       -o png/tunesync-logo-dark-1200.png
rsvg-convert -w 1200 -b '#ffffff' tunesync-logo-light.svg -o png/tunesync-logo-light-1200.png
python3 - <<'PY'
from PIL import Image
sizes = [16, 24, 32, 48, 64, 128, 256]
ims = {s: Image.open(f"png/tunesync-mark-{s}.png").convert("RGBA") for s in sizes}
# append_images keeps the per-size drawings instead of letting Pillow downscale the 256 px one
ims[256].save("tunesync.ico", format="ICO", sizes=[(s, s) for s in sizes],
              append_images=[ims[s] for s in sizes if s != 256])
PY
