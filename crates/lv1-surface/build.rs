// The crash guard around calls into LV1 is SEH (__try / __except), which Rust does not
// have: a few lines of C, Windows only.
fn main() {
    println!("cargo:rerun-if-changed=csrc/guard.c");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        cc::Build::new().file("csrc/guard.c").compile("lv1guard");
    }
}
