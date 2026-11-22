#!/bin/bash
# cc.sh - the linker for the Apple targets when building on Linux (tools/build-mac.sh sets
# it as CARGO_TARGET_<T>_LINKER): clang-18 driving ld64.lld, one architecture per call.
#
# There is no Apple SDK here, so every link runs twice:
#   1. a probe link with `-undefined dynamic_lookup`, to learn which system symbols the
#      output imports (llvm-nm -u);
#   2. tools/mac/stubs.sh sorts those into their libraries and writes link stubs (.tbd)
#      that name exactly them, and the real link resolves every import against the stubs,
#      two-level, the way Apple's linker would with the SDK. A symbol the stubs script
#      cannot place fails the build instead of guessing.
# Both architectures are ad-hoc signed by the linker (macOS on arm64 runs nothing
# unsigned); tools/build-mac.sh signs the finished bundle again with rcodesign.
#
# Env: TUNESYNC_STUB_SDK (where the stub SDKs go), MACOSX_DEPLOYMENT_TARGET (11.0).
set -euo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
arch=; out=; prev=; args=()
for a in "$@"; do
    case $prev in
        -arch) arch=$a ;;
        -o) out=$a ;;
    esac
    # rustc may pass SDKROOT as -isysroot: each architecture gets its own stub SDK instead
    if [ "$a" = -isysroot ] || [ "$prev" = -isysroot ]; then prev=$a; continue; fi
    args+=("$a"); prev=$a
done
case $arch in
    arm64|x86_64) ;;
    *) echo "cc.sh: no -arch arm64|x86_64 in the link line" >&2; exit 2 ;;
esac
[ -n "$out" ] || { echo "cc.sh: no -o in the link line" >&2; exit 2; }
min=${MACOSX_DEPLOYMENT_TARGET:-11.0}
sdk="${TUNESYNC_STUB_SDK:?set TUNESYNC_STUB_SDK}/$arch/MacOSX.sdk"
link() { clang-18 -target "$arch-apple-macos$min" -fuse-ld=lld -isysroot "$sdk" -Wl,-adhoc_codesign "$@"; }

# 1. probe: every import left to the loader, against empty stubs
"$HERE/stubs.sh" "$arch" "$sdk" /dev/null
probe_args=()
prev=
for a in "${args[@]}"; do
    if [ "$prev" = -o ]; then probe_args+=("$out.probe"); else probe_args+=("$a"); fi
    prev=$a
done
link "${probe_args[@]}" -Wl,-undefined,dynamic_lookup
llvm-nm-18 -u "$out.probe" | sort -u > "$out.imports"
rm -f "$out.probe"

# 2. the real link, against stubs naming exactly those imports
"$HERE/stubs.sh" "$arch" "$sdk" "$out.imports"
link "${args[@]}"
