#!/bin/bash
# build-dll.sh - TuneSync_Control.dll for Windows x64 (and the fake LV1 when it exists),
# built in the tunesync-build:1 image. Writes dist/TuneSync_Control.dll + dist/BUILD.txt.
#   tools/build-dll.sh [--debug]
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/.." && pwd)
PROFILE=release; FLAG=--release
[ "${1:-}" = --debug ] && { PROFILE=debug; FLAG=; }
T=x86_64-pc-windows-msvc
PKGS="-p tunesync-control"
[ -d "$ROOT/crates/sim" ] && PKGS="$PKGS -p tunesync-sim"
"$ROOT/tools/cargo.sh" build $FLAG --target $T $PKGS
OUT="$ROOT/dist"; mkdir -p "$OUT"
cp "$ROOT/target/$T/$PROFILE/tunesync_control.dll" "$OUT/TuneSync_Control.dll"
for f in lv1sim.exe MixerCoreSG.dll; do
    [ -f "$ROOT/target/$T/$PROFILE/$f" ] && cp "$ROOT/target/$T/$PROFILE/$f" "$OUT/"
done
[ -f "$ROOT/target/$T/$PROFILE/mixercoresg.dll" ] && cp "$ROOT/target/$T/$PROFILE/mixercoresg.dll" "$OUT/MixerCoreSG.dll"
COMMIT=$(git -C "$ROOT" rev-parse --short HEAD 2>/dev/null || echo nogit)
git -C "$ROOT" diff --quiet HEAD -- 2>/dev/null || COMMIT="$COMMIT+"
VER=$(sed -n 's/^version = "\(.*\)"/\1/p' "$ROOT/Cargo.toml" | head -1)
printf 'TuneSync %s\ncommit : %s\nbuilt  : %s (%s, tunesync-build:1)\n' "$VER" "$COMMIT" "$(date -u '+%Y-%m-%d %H:%M UTC')" "$PROFILE" > "$OUT/BUILD.txt"
ls -l "$OUT"
