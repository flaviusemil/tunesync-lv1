# tune-remote-rust:1 - Tune Remote's build image: Rust (Linux host target for the tests,
# x86_64-pc-windows-msvc for the LV1 DLL) on top of Headlink's headlink-xbuild:1, which
# brings clang-cl / lld-link / llvm-lib and Microsoft's CRT + Windows SDK (xwin, licence
# accepted by Flavius for headlink-xbuild on 2026-10-20).
#   docker build -t tune-remote-rust:1 tools/docker
FROM headlink-xbuild:1
ENV RUSTUP_HOME=/opt/rustup CARGO_HOME=/opt/cargo PATH=/opt/cargo/bin:$PATH
RUN apt-get update && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends gcc libc6-dev \
    && rm -rf /var/lib/apt/lists/* \
    && curl -fsSL https://sh.rustup.rs -o /tmp/rustup.sh \
    && sh /tmp/rustup.sh -y --no-modify-path --profile minimal --default-toolchain stable \
    && rustup target add x86_64-pc-windows-msvc \
    && rustup component add clippy rustfmt \
    && rm /tmp/rustup.sh && chmod -R a+rwX /opt/cargo /opt/rustup
# The MSVC target links with lld-link against xwin's CRT and SDK; the cc crate (the SEH
# shim) compiles with clang-cl against the same headers.
ENV CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER=lld-link \
    CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_RUSTFLAGS="-C target-feature=+crt-static -Lnative=/xwin/crt/lib/x86_64 -Lnative=/xwin/sdk/lib/um/x86_64 -Lnative=/xwin/sdk/lib/ucrt/x86_64" \
    CC_x86_64_pc_windows_msvc=clang-cl \
    AR_x86_64_pc_windows_msvc=llvm-lib \
    CFLAGS_x86_64_pc_windows_msvc="--target=x86_64-pc-windows-msvc /imsvc /xwin/crt/include /imsvc /xwin/sdk/include/ucrt /imsvc /xwin/sdk/include/um /imsvc /xwin/sdk/include/shared"
WORKDIR /src
