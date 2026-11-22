#!/bin/bash
# cargo.sh - run cargo in the tunesync-build:1 image (tools/docker/Dockerfile), with the
# registry cached in ~/.cache/tunesync-cargo. Examples:
#   tools/cargo.sh test                       the Linux tests
#   tools/cargo.sh build --release --target x86_64-pc-windows-msvc -p tr-lv1
#   IMAGE=tunesync-mac:1 tools/cargo.sh check --target aarch64-apple-darwin   (the Mac targets)
set -euo pipefail
IMAGE=${IMAGE:-tunesync-build:1}
ROOT=$(cd "$(dirname "$0")/.." && pwd)
CACHE=${TR_CARGO_CACHE:-$HOME/.cache/tunesync-cargo}
mkdir -p "$CACHE/registry" "$CACHE/git"
exec docker run --rm --user "$(id -u):$(id -g)" -e HOME=/tmp -e CARGO_TERM_COLOR=never \
    -v "$ROOT":/src -v "$CACHE/registry":/opt/cargo/registry -v "$CACHE/git":/opt/cargo/git \
    ${CARGO_ENV:-} -w /src "$IMAGE" cargo "$@"
