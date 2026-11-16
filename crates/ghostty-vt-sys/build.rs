use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    let root = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap()).join("../..");
    let script = root.join("scripts/ghostty/build-lib.sh");
    println!("cargo:rerun-if-changed={}", script.display());
    println!(
        "cargo:rerun-if-changed={}",
        root.join("scripts/ghostty/build-xcframework.sh").display()
    );
    let target = env::var("TARGET").unwrap();
    assert!(
        target == "aarch64-apple-darwin",
        "libghostty-vt is built for aarch64-apple-darwin only in this prototype, not {target} (#764)"
    );
    let status = Command::new(&script).status().unwrap();
    assert!(status.success(), "{} failed", script.display());
    let lib = root.join("target/ghostty/collied/lib");
    println!("cargo:rustc-link-search=native={}", lib.display());
    println!("cargo:rustc-link-lib=static=ghostty-vt");
}
