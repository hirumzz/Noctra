use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=noctra.rc");
    println!("cargo:rerun-if-changed=assets/noctra.ico");

    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let target_env = env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();

    if target_os == "windows" && target_env == "gnu" {
        let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
        let obj_path = out_dir.join("noctra_res.o");

        let status = Command::new("x86_64-w64-mingw32-windres")
            .args(["noctra.rc", "-O", "coff", "-o"])
            .arg(&obj_path)
            .status();

        if let Ok(s) = status {
            if s.success() {
                println!("cargo:rustc-link-arg-bins={}", obj_path.display());
            }
        }
    }
}
