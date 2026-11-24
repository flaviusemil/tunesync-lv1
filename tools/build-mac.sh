#!/bin/bash
# build-mac.sh - TuneSync for eMotion LV1 on macOS: TuneSync_Control.bundle, universal
# (x86_64 + arm64) and ad-hoc signed, and the zip that installs it.
#   tools/build-mac.sh [--debug]
#
# On Linux it runs itself in the tunesync-mac:1 image (tools/docker/mac.Dockerfile):
# cargo for both Apple targets with tools/mac/cc.sh as the linker (no Apple SDK: it links
# against stubs of exactly the system symbols the bundle imports), llvm-lipo, rcodesign.
# On a Mac it uses the Xcode command-line tools instead (rustup targets
# x86_64-apple-darwin and aarch64-apple-darwin installed).
#
# Writes dist/mac/:
#   TuneSync_Control.bundle      for /Applications/Waves/eMotion LV1/Modules
#   TuneSync-<version>-macos.zip the bundle, install.sh, lv1-prefs.sh, README.txt
#   rtpsend                      the stand-in playback rig (universal), for the rig test
#   sim/lv1sim.app, sim/libmixercoresg.dylib
#                                the fake LV1 for Mac (tools/mac/run-sim.sh runs it there)
#   BUILD.txt                    version, commit, time, sha256
# Nothing is published.
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/.." && pwd)
PROFILE=release; FLAG=--release
[ "${1:-}" = --debug ] && { PROFILE=debug; FLAG=; }
VER=$(sed -n 's/^version = "\(.*\)"/\1/p' "$ROOT/Cargo.toml" | head -1)
MINOS=11.0

if [ "$(uname)" != Darwin ] && [ -z "${TUNESYNC_IN_IMAGE:-}" ]; then
    CACHE=${TR_CARGO_CACHE:-$HOME/.cache/tunesync-cargo}
    mkdir -p "$CACHE/registry" "$CACHE/git"
    COMMIT=$(git -C "$ROOT" rev-parse --short HEAD 2>/dev/null || echo nogit)
    git -C "$ROOT" diff --quiet HEAD -- 2>/dev/null || COMMIT="$COMMIT+"
    exec docker run --rm --user "$(id -u):$(id -g)" -e HOME=/tmp -e CARGO_TERM_COLOR=never \
        -e TUNESYNC_IN_IMAGE=1 -e TUNESYNC_COMMIT="$COMMIT" \
        -v "$ROOT":/src -v "$CACHE/registry":/opt/cargo/registry -v "$CACHE/git":/opt/cargo/git \
        -w /src tunesync-mac:1 tools/build-mac.sh "$@"
fi

export MACOSX_DEPLOYMENT_TARGET=$MINOS
if [ "$(uname)" = Darwin ]; then
    LIPO=lipo
    COMMIT=$(git -C "$ROOT" rev-parse --short HEAD 2>/dev/null || echo nogit)
    git -C "$ROOT" diff --quiet HEAD -- 2>/dev/null || COMMIT="$COMMIT+"
    HOW="Xcode command-line tools"
else
    LIPO=llvm-lipo-18
    COMMIT=${TUNESYNC_COMMIT:-nogit}
    HOW="tunesync-mac:1, link stubs"
    export TUNESYNC_STUB_SDK=$ROOT/target/mac-sdk
    export CARGO_TARGET_X86_64_APPLE_DARWIN_LINKER=$ROOT/tools/mac/cc.sh
    export CARGO_TARGET_AARCH64_APPLE_DARWIN_LINKER=$ROOT/tools/mac/cc.sh
    # rustc only wants an SDK to hand to the linker; cc.sh uses its own stubs instead.
    mkdir -p "$TUNESYNC_STUB_SDK"
    export SDKROOT=$TUNESYNC_STUB_SDK
fi

TARGETS="x86_64-apple-darwin aarch64-apple-darwin"
for t in $TARGETS; do
    (cd "$ROOT" && cargo build $FLAG --target "$t" -p tunesync-control)
    (cd "$ROOT" && cargo build $FLAG --target "$t" -p rtpmidi --example rtpsend)
    (cd "$ROOT" && cargo build $FLAG --target "$t" -p tunesync-sim)
done

OUT=$ROOT/dist/mac
B=$OUT/TuneSync_Control.bundle
SIM=$OUT/sim
rm -rf "$B" "$OUT"/TuneSync-*-macos "$OUT"/TuneSync-*-macos.zip "$SIM"
mkdir -p "$B/Contents/MacOS" "$B/Contents/Resources" "$SIM/lv1sim.app/Contents/MacOS"
slices=(); rigs=(); sims=(); cores=()
for t in $TARGETS; do
    slices+=("$ROOT/target/$t/$PROFILE/libtunesync_control.dylib")
    rigs+=("$ROOT/target/$t/$PROFILE/examples/rtpsend")
    sims+=("$ROOT/target/$t/$PROFILE/lv1sim")
    cores+=("$ROOT/target/$t/$PROFILE/libmixercoresg.dylib")
done
$LIPO -create "${slices[@]}" -output "$B/Contents/MacOS/TuneSync_Control"
$LIPO -create "${rigs[@]}" -output "$OUT/rtpsend"
$LIPO -create "${sims[@]}" -output "$SIM/lv1sim.app/Contents/MacOS/lv1sim"
$LIPO -create "${cores[@]}" -output "$SIM/libmixercoresg.dylib"
# An app, so `open` can start it in the logged-in user's window session (from ssh too).
cat > "$SIM/lv1sim.app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
    <key>CFBundleExecutable</key><string>lv1sim</string>
    <key>CFBundleIdentifier</key><string>ro.oriontechnologies.tunesync.lv1sim</string>
    <key>CFBundleName</key><string>lv1sim</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleShortVersionString</key><string>$VER</string>
    <key>LSMinimumSystemVersion</key><string>$MINOS</string>
    <key>LSUIElement</key><true/>
</dict></plist>
PLIST
sed -e "s/@VERSION@/$VER/g" -e "s/@MINOS@/$MINOS/g" "$ROOT/installer/mac/Info.plist" > "$B/Contents/Info.plist"
cp "$ROOT/LICENSE-MIT" "$ROOT/LICENSE-APACHE" "$B/Contents/Resources/"
# The picture LV1 shows on TuneSync's tile (Setup > System Inventory > CONTROLS): the bundle
# sends its path when the file is in Resources; Qt takes the @2x on a Retina screen.
cp "$ROOT"/assets/lv1-surface/tunesync-surface.png "$ROOT"/assets/lv1-surface/tunesync-surface@2x.png "$B/Contents/Resources/"

# Each slice must export LV1's 18 WMSD_* functions (and nothing of ours besides).
if [ "$(uname)" = Darwin ]; then NM=nm; else NM=llvm-nm-18; fi
for a in x86_64 arm64; do
    n=$($NM -gU -arch "$a" "$B/Contents/MacOS/TuneSync_Control" 2>/dev/null | grep -c ' _WMSD_' || true)
    [ "$n" = 18 ] || { echo "build-mac.sh: the $a slice exports $n WMSD_ functions, not 18" >&2; exit 1; }
done

# Ad-hoc signature over the whole bundle (seals Info.plist and the resources).
if [ "$(uname)" = Darwin ]; then
    codesign --force --sign - --timestamp=none "$B"
    codesign --verify --strict --verbose=1 "$B"
    for f in "$OUT/rtpsend" "$SIM/libmixercoresg.dylib" "$SIM/lv1sim.app"; do codesign --force --sign - --timestamp=none "$f"; done
else
    sign() { rcodesign "$@" > "$OUT/sign.log" 2>&1 || { cat "$OUT/sign.log" >&2; exit 1; }; }
    sign sign "$B"
    # (rcodesign's verify misreads ad-hoc signatures: check what codesign -s - would write
    # instead, in both slices: ad-hoc, sealing Info.plist and the resources)
    info=$(rcodesign print-signature-info "$B/Contents/MacOS/TuneSync_Control" 2>/dev/null)
    for want in 'flags: CodeSignatureFlags(ADHOC)' "'Info (1)" "'Resources (3)" 'identifier: ro.oriontechnologies.tunesync.control'; do
        [ "$(grep -cF "$want" <<<"$info")" = 2 ] || { echo "build-mac.sh: the bundle's signature lacks \"$want\" in a slice" >&2; exit 1; }
    done
    for f in "$OUT/rtpsend" "$SIM/libmixercoresg.dylib" "$SIM/lv1sim.app"; do sign sign "$f"; done
    rm -f "$OUT/sign.log"
fi

# The zip: what a user downloads.
STAGE=$OUT/TuneSync-$VER-macos
mkdir -p "$STAGE"
cp -R "$B" "$STAGE/"
cp "$ROOT/installer/mac/install.sh" "$ROOT/installer/mac/lv1-prefs.sh" "$STAGE/"
sed "s/@VERSION@/$VER/g" "$ROOT/installer/mac/README.txt" > "$STAGE/README.txt"
chmod 755 "$STAGE/install.sh" "$STAGE/lv1-prefs.sh"
if [ "$(uname)" = Darwin ]; then
    (cd "$OUT" && ditto -c -k --keepParent "TuneSync-$VER-macos" "TuneSync-$VER-macos.zip")
else
    python3 -I - "$OUT" "TuneSync-$VER-macos" <<'PY'
import os, sys, zipfile
out, name = sys.argv[1], sys.argv[2]
with zipfile.ZipFile(os.path.join(out, name + ".zip"), "w", zipfile.ZIP_DEFLATED) as z:
    for d, dirs, files in os.walk(os.path.join(out, name)):
        dirs.sort()
        rel = os.path.relpath(d, out)
        z.write(d, rel)  # directory entries keep their modes
        for f in sorted(files):
            z.write(os.path.join(d, f), os.path.join(rel, f))
PY
fi
rm -rf "$STAGE"

SHA=$(sha256sum "$B/Contents/MacOS/TuneSync_Control" 2>/dev/null || shasum -a 256 "$B/Contents/MacOS/TuneSync_Control")
printf 'TuneSync %s for macOS (x86_64 + arm64, macOS %s or later)\ncommit : %s\nbuilt  : %s (%s, %s)\nsha256 : %s\n' \
    "$VER" "$MINOS" "$COMMIT" "$(date -u '+%Y-%m-%d %H:%M UTC')" "$PROFILE" "$HOW" "${SHA%% *}" > "$OUT/BUILD.txt"
cat "$OUT/BUILD.txt"
ls -l "$OUT"
