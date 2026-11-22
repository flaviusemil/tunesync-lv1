// The DLL's version resource (Windows): written here with the crate's version and
// compiled with llvm-rc (no preprocessor needed), then handed to lld-link. Skipped with a
// warning where llvm-rc is missing.
use std::process::Command;

fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let ver = std::env::var("CARGO_PKG_VERSION").unwrap();
    let n: Vec<&str> = ver.split('.').collect();
    let rc = format!(
        r#"1 VERSIONINFO
 FILEVERSION {a},{b},{c},0
 PRODUCTVERSION {a},{b},{c},0
 FILEOS 0x40004
 FILETYPE 0x2
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "040904b0"
    BEGIN
      VALUE "CompanyName", "Orion Technologies"
      VALUE "FileDescription", "TuneSync - MIDI sets the key of every Waves Tune Real-Time in eMotion LV1"
      VALUE "FileVersion", "{ver}"
      VALUE "InternalName", "TuneSync_Control"
      VALUE "OriginalFilename", "TuneSync_Control.dll"
      VALUE "ProductName", "TuneSync"
      VALUE "ProductVersion", "{ver}"
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x409, 1200
  END
END
"#,
        a = n[0],
        b = n[1],
        c = n[2]
    );
    let src = out.join("version.rc");
    let res = out.join("version.res");
    std::fs::write(&src, rc).unwrap();
    match Command::new("llvm-rc").arg("-no-preprocess").arg("/fo").arg(&res).arg(&src).status() {
        Ok(s) if s.success() => println!("cargo:rustc-link-arg={}", res.display()),
        _ => println!("cargo:warning=llvm-rc failed or is missing: the DLL has no version resource"),
    }
}
