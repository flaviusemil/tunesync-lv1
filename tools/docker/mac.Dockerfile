# tunesync-mac:1 - the macOS bundle built on Linux: tunesync-build:1 plus the two Apple
# Rust targets, ld64.lld as clang's Mach-O linker, and rcodesign (apple-codesign, MPL-2.0)
# for the bundle's ad-hoc signature. No Apple SDK: tools/mac/stubs.sh writes link stubs
# (.tbd) naming only the system symbols TuneSync imports, see tools/build-mac.sh.
#   docker build -t tunesync-mac:1 -f tools/docker/mac.Dockerfile tools/docker
FROM tunesync-build:1
RUN rustup target add x86_64-apple-darwin aarch64-apple-darwin \
    && ln -s /usr/bin/ld64.lld-18 /usr/local/bin/ld64.lld \
    && cargo install apple-codesign --version =0.29.0 --locked --no-default-features --root /opt/rcodesign \
    && ln -s /opt/rcodesign/bin/rcodesign /usr/local/bin/rcodesign \
    && rm -rf /opt/cargo/registry/cache /opt/cargo/registry/src \
    && chmod -R a+rwX /opt/cargo /opt/rustup
