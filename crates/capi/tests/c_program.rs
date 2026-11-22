//! The C API from C: builds tests/c/capi_test.c with clang (`$CC` overrides) against
//! include/tunesync.h and this crate's staticlib, and runs it against a fake LV1 vtable.
//! Linux only (the build image); the header is also checked as C++.
#![cfg(target_os = "linux")]
use std::path::{Path, PathBuf};
use std::process::Command;

fn compiler() -> String {
    if let Some(cc) = std::env::var("CC").ok().filter(|c| !c.is_empty()) {
        return cc;
    }
    ["clang-18", "clang"].into_iter().find(|c| Command::new(c).arg("--version").output().is_ok_and(|o| o.status.success())).unwrap_or("clang").to_string()
}

fn manifest() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

// The staticlib cargo built for this test run. `cargo build` leaves it in target/<profile>;
// `cargo test` (1.100's build-dir layout) only in target/<profile>/build/tunesync-capi/
// <hash>/out/ and does not copy it up, so the newest of every copy above this test's
// binary is the one built from the current source.
fn staticlib() -> PathBuf {
    const NAME: &str = "libtunesync_capi.a";
    let exe = std::env::current_exe().unwrap();
    let mut found = Vec::new();
    for d in exe.ancestors().skip(1).take(6) {
        found.push(d.join(NAME));
        found.push(d.join("deps").join(NAME));
        if let Ok(rd) = std::fs::read_dir(d.join("build").join("tunesync-capi")) {
            found.extend(rd.flatten().map(|e| e.path().join("out").join(NAME)));
        }
    }
    found
        .into_iter()
        .filter_map(|p| Some((std::fs::metadata(&p).ok()?.modified().ok()?, p)))
        .max()
        .map(|(_, p)| p)
        .unwrap_or_else(|| panic!("{NAME} not found above {}", exe.display()))
}

fn run(cmd: &mut Command) -> String {
    let out = cmd.output().unwrap_or_else(|e| panic!("{cmd:?}: {e}"));
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success(), "{cmd:?} failed ({}):\n{text}", out.status);
    text
}

#[test]
fn c_program_drives_the_api() {
    let tmp = PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
    let exe = tmp.join("capi_test");
    run(Command::new(compiler())
        .args(["-std=c11", "-Wall", "-Wextra", "-Werror", "-pedantic", "-g", "-I"])
        .arg(manifest().join("include"))
        .arg(manifest().join("tests/c/capi_test.c"))
        .arg(staticlib())
        // What `rustc --print native-static-libs` lists for a std staticlib on glibc.
        .args(["-lgcc_s", "-lutil", "-lrt", "-lpthread", "-lm", "-ldl", "-lc", "-o"])
        .arg(&exe));
    let dir = tmp.join(format!("capi-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let out = run(Command::new(&exe).arg(&dir));
    println!("{out}");
    assert!(out.contains("ok: 0 failures"), "{out}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn header_is_valid_cpp() {
    run(Command::new(compiler()).args(["-x", "c++", "-std=c++17", "-fsyntax-only", "-Wall", "-Wextra", "-Werror", "-pedantic"]).arg(manifest().join("include/tunesync.h")));
}
