#!/bin/bash
# run-sim.sh - the fake LV1 for Mac on a Mac, over SSH: TuneSync_Control.bundle and
# dist/mac/sim (tools/build-mac.sh) go to a fresh folder there, lv1sim runs, and its
# report and the window's PNGs come back to dist/mac/sim-run/. LV1 is not touched and
# nothing is installed; the folder on the Mac is removed afterwards.
#
#   MAC=user@host [MAC_JUMP=user@jump:port] tools/mac/run-sim.sh [--window <s>] [--headless]
#
# By default lv1sim is started with `open` in the logged-in user's window session, so the
# settings window can open and be rendered (someone must be logged in at the Mac; the
# report comes back through files). --headless runs it straight from the SSH session
# instead (no window: SSH sessions cannot reach the window server).
# Exit code 0 = every check passed.
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
M=${MAC:?set MAC=user@host of the Mac}
SSH=(ssh -o BatchMode=yes -o ConnectTimeout=20)
[ -n "${MAC_JUMP:-}" ] && SSH+=(-J "$MAC_JUMP")
WIN=8; HEADLESS=0
while [ $# -gt 0 ]; do
    case $1 in
        --window) WIN=$2; shift ;;
        --headless) HEADLESS=1 ;;
        *) sed -n '2,13s/^# \{0,1\}//p' "$0"; exit 1 ;;
    esac
    shift
done
D=$ROOT/dist/mac
[ -d "$D/sim/lv1sim.app" ] && [ -d "$D/TuneSync_Control.bundle" ] || { echo "no dist/mac/sim: run tools/build-mac.sh first" >&2; exit 1; }
R=tunesync-sim-$(date +%Y%m%d-%H%M%S)
OUT=$D/sim-run; rm -rf "$OUT"; mkdir -p "$OUT"

echo "== copying to $M:~/$R"
tar -C "$D" -cf - TuneSync_Control.bundle sim rtpsend | "${SSH[@]}" "$M" "mkdir -p ~/$R && tar -xf - -C ~/$R"

echo "== running lv1sim"
set +e
"${SSH[@]}" "$M" bash -s -- "$R" "$WIN" "$HEADLESS" <<'REMOTE'
R=$1; WIN=$2; HEADLESS=$3
cd ~/"$R" || exit 1
echo "mac: macOS $(sw_vers -productVersion) on $(uname -m)"
ARGS=(--bundle "$PWD/TuneSync_Control.bundle" --core "$PWD/sim/libmixercoresg.dylib")
if [ "$HEADLESS" = 1 ]; then
    ./sim/lv1sim.app/Contents/MacOS/lv1sim "${ARGS[@]}" --headless > out.txt 2> err.txt
else
    open -W -n --stdout "$PWD/out.txt" --stderr "$PWD/err.txt" ./sim/lv1sim.app --args "${ARGS[@]}" --window "$WIN" --shot "$PWD/shot"
fi
cat out.txt
[ -s err.txt ] && { echo "---- stderr"; cat err.txt; }
grep -q 'lv1sim: [0-9]* passed, 0 failed' out.txt
REMOTE
rc=$?
set -e

echo "== fetching the report"
"${SSH[@]}" "$M" "cd ~/$R && tar -cf - out.txt err.txt shot-*.png 2>/dev/null" | tar -xf - -C "$OUT" 2>/dev/null || true
"${SSH[@]}" "$M" "rm -rf ~/$R"
ls -l "$OUT"
[ $rc = 0 ] && echo "run-sim: every check passed" || echo "run-sim: FAILED (see $OUT/out.txt)"
exit $rc
