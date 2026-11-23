#!/bin/sh
# Regenerates the PNGs LV1 loads from tunesync-surface.svg. Needs rsvg-convert.
# LV1 stretches the picture over a 100x64 frame and, on a HiDPI screen, picks the @2x file
# next to the one named (Qt's stylesheet image lookup), so both sizes ship side by side.
set -eu
cd "$(dirname "$0")"
rsvg-convert -w 200 -h 128 tunesync-surface.svg -o tunesync-surface@2x.png
rsvg-convert -w 100 -h 64  tunesync-surface.svg -o tunesync-surface.png
