# tunesync-xwin:1 - the base TuneSync's build image sits on: clang-cl / lld-link / llvm-lib
# (LLVM 18) and Microsoft's CRT + Windows SDK, fetched by xwin, so the Windows MSVC target
# links on Linux.
#
#   docker build -t tunesync-xwin:1 -f tools/docker/xwin.Dockerfile tools/docker
#
# Building it downloads the Microsoft CRT and Windows SDK, and `xwin --accept-license`
# accepts Microsoft's licence for them (https://go.microsoft.com/fwlink/?LinkId=2086102).
# Build it only if you accept that licence yourself. (Headlink's headlink-xbuild:1 is the
# same base; tools/docker/Dockerfile takes either through --build-arg BASE=.)
FROM ubuntu:24.04
RUN apt-get update && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
        clang-18 lld-18 llvm-18 ca-certificates curl python3 make git \
    && rm -rf /var/lib/apt/lists/*
RUN for t in clang-cl lld-link llvm-ml llvm-rc llvm-lib llvm-mt; do ln -sf /usr/bin/$t-18 /usr/local/bin/$t 2>/dev/null || true; done \
    && ln -sf /usr/bin/clang-18 /usr/local/bin/clang-cl && ln -sf /usr/bin/lld-18 /usr/local/bin/lld-link
ARG XWIN=0.6.5
RUN curl -fsSL https://github.com/Jake-Shadle/xwin/releases/download/$XWIN/xwin-$XWIN-x86_64-unknown-linux-musl.tar.gz \
      | tar -xz -C /usr/local/bin --strip-components=1 xwin-$XWIN-x86_64-unknown-linux-musl/xwin \
    && xwin --accept-license --arch x86_64 splat --output /xwin \
    && rm -rf .xwin-cache
ENV XWIN=/xwin
WORKDIR /src
