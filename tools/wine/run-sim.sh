#!/bin/bash
# run-sim.sh - run dist/lv1sim.exe (the fake LV1 driving TuneSync_Control.dll) under Wine,
# in Headlink's headlink-inno:1 image (Wine 8 + Xvfb). Test only.
#   tools/wine/run-sim.sh [--window <seconds>] [--shot <png>]
# --shot takes a screenshot of the virtual screen after the window opened.
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
RUN=$(mktemp -d); trap 'rm -rf "$RUN"' EXIT
cp "$ROOT"/dist/{lv1sim.exe,MixerCoreSG.dll,TuneSync_Control.dll} "$RUN/"
# Wine 8 has no bcryptprimitives.dll (ProcessPrng): a stub beside the exe, for this run only.
# (Relative paths: clang-cl reads "/w..." as its own /w flag.)
docker run --rm --user "$(id -u):$(id -g)" -v "$ROOT/tools/wine":/src/wine -v "$RUN":/src/out -w /src tunesync-build:1 bash -c \
  'clang-cl --target=x86_64-pc-windows-msvc /nologo /LD /O2 /imsvc /xwin/crt/include /imsvc /xwin/sdk/include/ucrt /imsvc /xwin/sdk/include/um /imsvc /xwin/sdk/include/shared wine/bcryptprimitives.c /Foout/b.obj /Feout/bcryptprimitives.dll -fuse-ld=lld-link /link /libpath:/xwin/crt/lib/x86_64 /libpath:/xwin/sdk/lib/um/x86_64 /libpath:/xwin/sdk/lib/ucrt/x86_64 bcrypt.lib >/dev/null && rm -f out/b.obj out/bcryptprimitives.lib out/bcryptprimitives.exp'
# --shot <dir>: lv1sim captures the window (page, RTP-MIDI sheet) as BMPs; they land in <dir>.
ARGS=(); SHOTDIR=""
while [ $# -gt 0 ]; do case "$1" in --shot) SHOTDIR=$2; ARGS+=(--shot shot); shift;; *) ARGS+=("$1");; esac; shift; done
set +e
# FONTS=1: install a few Linux fonts in the throwaway container (Wine has no Segoe UI).
docker run --rm -e FONTS="${FONTS:-}" -v "$RUN":/o -w /o --entrypoint bash headlink-inno:1 -c '
  [ -n "$FONTS" ] && { apt-get update -qq >/dev/null && apt-get install -y -qq fonts-dejavu-core fonts-liberation >/dev/null 2>&1; }
  Xvfb :9 -screen 0 1280x1000x24 >/dev/null 2>&1 & export DISPLAY=:9; sleep 1
  wine lv1sim.exe "$@" 2>/dev/null; rc=$?; chmod -R a+rw /o; exit $rc' bash "${ARGS[@]}"
rc=$?
set -e
if [ -n "$SHOTDIR" ]; then mkdir -p "$SHOTDIR"; cp "$RUN"/shot-*.bmp "$SHOTDIR"/ 2>/dev/null || true; fi
exit $rc
