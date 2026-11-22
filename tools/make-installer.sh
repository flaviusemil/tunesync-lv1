#!/bin/bash
# make-installer.sh [--sign] - TuneSync-<version>-setup.exe from dist/ (tools/build-dll.sh
# first), compiled with Inno Setup under Wine in Headlink's headlink-inno:1 image.
#   --sign   Authenticode-signs the DLL, the setup and its uninstaller (osslsigncode +
#            RFC 3161 timestamp) with the PEM key and certificate named by
#            $TUNESYNC_CODESIGN_KEY and $TUNESYNC_CODESIGN_CERT (mounted read-only for the
#            build only; never commit them). Without --sign nothing is signed.
# Writes dist/TuneSync-<version>-setup.exe. Nothing is published.
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/.." && pwd)
SIGN=0; [ "${1:-}" = --sign ] && SIGN=1
VER=$(sed -n 's/^version = "\(.*\)"/\1/p' "$ROOT/Cargo.toml" | head -1)
BUILD="$(git -C "$ROOT" rev-parse --short HEAD 2>/dev/null || echo nogit) $(date -u +%Y%m%d.%H%M%S)"
[ -f "$ROOT/dist/TuneSync_Control.dll" ] || { echo "no dist/TuneSync_Control.dll: run tools/build-dll.sh first" >&2; exit 1; }
SIGNARGS=(); MOUNTS=()
if [ $SIGN = 1 ]; then
    for k in "${TUNESYNC_CODESIGN_KEY:-}" "${TUNESYNC_CODESIGN_CERT:-}"; do [ -n "$k" ] && [ -f "$k" ] || { echo "--sign needs TUNESYNC_CODESIGN_KEY and TUNESYNC_CODESIGN_CERT (PEM files)" >&2; exit 1; }; done
    MOUNTS=(-v "$TUNESYNC_CODESIGN_KEY:/keys/codesign.key:ro" -v "$TUNESYNC_CODESIGN_CERT:/keys/codesign.crt:ro"
            -v "$ROOT/tools/tssign:/usr/local/bin/hl-sign:ro")
    SIGNARGS=('/DSign=1' '/Stssign=cmd /c Z:\usr\local\bin\hl-sign.bat $f')
    # The DLL first (osslsigncode in the same image).
    docker run --rm -e HOST_UID="$(id -u)" "${MOUNTS[@]}" -v "$ROOT/dist":/work --entrypoint sh headlink-inno:1 -c \
        'printf "%s" "Z:\\work\\TuneSync_Control.dll" > /tmp/p; /usr/local/bin/hl-sign "$(cat /tmp/p)"; cat /work/sign.done; rm -f /work/sign.done; chown $HOST_UID /work/*'
fi
docker run --rm -e HOST_UID="$(id -u)" -e HOST_GID="$(id -g)" "${MOUNTS[@]}" \
    -v "$ROOT/installer":/src/installer:ro -v "$ROOT/dist":/dist:ro -v "$ROOT/dist":/work \
    headlink-inno:1 iscc /src/installer/tunesync.iss "/DAppVersion=$VER" "/DBuild=$BUILD" '/DDist=Z:\dist' '/DOutDir=Z:\work' "${SIGNARGS[@]}"
rm -f "$ROOT/dist/sign.log" "$ROOT/dist/sign.req" "$ROOT/dist/sign.done"
ls -l "$ROOT/dist/TuneSync-$VER-setup.exe"
