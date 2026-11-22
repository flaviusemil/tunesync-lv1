#!/bin/bash
# capi-header.sh - regenerate crates/capi/include/tunesync.h from crates/capi/src/lib.rs
# with cbindgen, in the tunesync-build:1 image (cbindgen is installed once into
# ~/.cache/tunesync-cargo/tools).
#   tools/capi-header.sh           write the header
#   tools/capi-header.sh --check   fail when the committed header is out of date
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/.." && pwd)
CACHE=${TR_CARGO_CACHE:-$HOME/.cache/tunesync-cargo}
CBINDGEN=0.27.0
VERIFY=
[ "${1:-}" = "--check" ] && VERIFY=--verify
mkdir -p "$CACHE/registry" "$CACHE/git" "$CACHE/tools"
exec docker run --rm --user "$(id -u):$(id -g)" -e HOME=/tmp -e CARGO_TERM_COLOR=never \
    -e CBINDGEN="$CBINDGEN" -e VERIFY="$VERIFY" \
    -v "$ROOT":/src -v "$CACHE/registry":/opt/cargo/registry -v "$CACHE/git":/opt/cargo/git \
    -v "$CACHE/tools":/opt/tools -w /src tunesync-build:1 sh -ec '
        [ "$(/opt/tools/bin/cbindgen --version 2>/dev/null)" = "cbindgen $CBINDGEN" ] \
            || cargo install --locked --root /opt/tools cbindgen --version "$CBINDGEN"
        /opt/tools/bin/cbindgen $VERIFY --config crates/capi/cbindgen.toml \
            --output crates/capi/include/tunesync.h crates/capi
        if [ -n "$VERIFY" ]; then echo "crates/capi/include/tunesync.h: up to date"; else echo "crates/capi/include/tunesync.h: written"; fi'
