#!/bin/bash
# xbuild.sh - Tune Remote's Windows build on Linux, with clang-cl + lld-link against
# Microsoft's CRT and SDK (xwin), inside Headlink's headlink-xbuild:1 image:
#
#   tools/xbuild.sh [--out dist]            from the repo root; runs the container itself
#
# Writes <out>/TuneRemote_Control.dll, <out>/lv1sim.exe (the fake-LV1 harness) and
# <out>/BUILD.txt. Static CRT (/MT): the DLL needs nothing installed beside LV1.
# Not signed: tools/sign.sh does that.
set -euo pipefail
OUT=dist
while [ $# -gt 0 ]; do case "$1" in --out) OUT=$2; shift;; --in-container) IN=1;; *) echo "unknown $1"; exit 2;; esac; shift; done
ROOT=$(cd "$(dirname "$0")/.." && pwd)
if [ -z "${IN:-}" ]; then
    COMMIT=$(git -C "$ROOT" rev-parse --short HEAD 2>/dev/null || echo nogit)
    git -C "$ROOT" diff --quiet HEAD -- 2>/dev/null || COMMIT="$COMMIT+"
    exec docker run --rm --user "$(id -u):$(id -g)" -e COMMIT="$COMMIT" -v "$ROOT":/src -w /src headlink-xbuild:1 \
        bash /src/tools/xbuild.sh --in-container --out "$OUT"
fi
X=${XWIN:-/xwin}
export HOME=/tmp
mkdir -p /tmp/xb && ln -sf /usr/bin/clang-18 /tmp/xb/clang && export PATH=/tmp/xb:$PATH   # llvm-rc preprocesses with "clang"
INC="/imsvc $X/crt/include /imsvc $X/sdk/include/ucrt /imsvc $X/sdk/include/um /imsvc $X/sdk/include/shared"
LIBP="/libpath:$X/crt/lib/x86_64 /libpath:$X/sdk/lib/um/x86_64 /libpath:$X/sdk/lib/ucrt/x86_64"
# The newest MSVC STL wants Clang 19; the image has 18 (headlink-xbuild trap).
CL="clang-cl --target=x86_64-pc-windows-msvc -fuse-ld=lld-link /nologo /EHa /O2 /MT /utf-8 /std:c++17 /W3 \
    -Wno-unused-command-line-argument -Wno-microsoft-cast /D_ALLOW_COMPILER_AND_STL_VERSION_MISMATCH \
    /DWIN32_LEAN_AND_MEAN /DNOMINMAX /D_WIN32_WINNT=0x0A00 /DUNICODE /D_UNICODE $INC"
RCI="/I $X/sdk/include/um /I $X/sdk/include/shared /I $X/crt/include /I $X/sdk/include/ucrt"
CORE="core/tune.cpp core/midi_map.cpp core/apply.cpp core/settings.cpp core/log.cpp core/engine.cpp"
NET="net/rtp_codec.cpp net/rtp_session.cpp"
UI=""; [ -f ui/view.cpp ] && UI="ui/view.cpp"
WIN="win/dll.cpp win/app.cpp win/lv1_mixer.cpp win/midi_in.cpp win/dnssd.cpp"
if [ -f win/window.cpp ]; then WIN="$WIN win/window.cpp"; else WIN="$WIN win/window_stub.cpp"; fi
LIBS="ws2_32.lib winmm.lib dnsapi.lib user32.lib gdi32.lib shell32.lib ole32.lib advapi32.lib d2d1.lib dwrite.lib dwmapi.lib"
B=build/win; mkdir -p "$B/obj" "$B/sim" "$OUT"
echo "=== TuneRemote_Control.dll"
llvm-rc $RCI /I win /fo "$B/version.res" win/version.rc
$CL /LD $CORE $NET $UI $WIN "$B/version.res" /Fo"$B/obj/" /Fe:"$B/TuneRemote_Control.dll" /link $LIBP $LIBS
if [ -f win/sim/lv1sim.cpp ]; then
    echo "=== lv1sim.exe + MixerCoreSG.dll (fake LV1)"
    $CL /LD win/sim/fakecore.cpp /Fo"$B/sim/" /Fe:"$B/sim/MixerCoreSG.dll" /link $LIBP
    $CL win/sim/lv1sim.cpp /Fo"$B/sim/" /Fe:"$B/sim/lv1sim.exe" /link $LIBP "$B/sim/MixerCoreSG.lib" ws2_32.lib user32.lib
fi
cp "$B/TuneRemote_Control.dll" "$OUT/"
[ -f "$B/sim/lv1sim.exe" ] && cp "$B/sim/lv1sim.exe" "$B/sim/MixerCoreSG.dll" "$OUT/"
printf 'Tune Remote %s\ncommit : %s\nbuilt  : %s (xbuild, clang-cl)\n' \
    "$(sed -n 's/^#define TR_VERSION_STR *"\(.*\)"/\1/p' win/version.h)" "${COMMIT:-nogit}" \
    "$(date -u '+%Y-%m-%d %H:%M UTC')" > "$OUT/BUILD.txt"
ls -l "$OUT"
