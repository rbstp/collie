use std::env;
use std::io;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    let root = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap()).join("../..");
    let scripts = root.join("scripts/ghostty");
    let target = env::var("TARGET").unwrap();

    println!("cargo:rerun-if-changed=build.rs");
    for file in ["pins.env", "common.sh", "build-collied.sh"] {
        println!("cargo:rerun-if-changed={}", scripts.join(file).display());
    }

    // Cargo reads the build script's stdout as directives, so the build's log goes to stderr.
    let script = scripts.join("build-collied.sh");
    let status = Command::new(&script)
        .arg(&target)
        .stdout(io::stderr())
        .status()
        .unwrap_or_else(|e| panic!("{}: {e}", script.display()));
    assert!(
        status.success(),
        "{} {target} failed with {status}",
        script.display()
    );

    let lib = root
        .join("target/ghostty/collied")
        .join(&target)
        .join("lib");
    // The archive lives outside OUT_DIR (CI caches it apart from target/): a missing one reruns this.
    println!(
        "cargo:rerun-if-changed={}",
        lib.join("libghostty-vt.a").display()
    );
    println!("cargo:rustc-link-search=native={}", lib.display());
    println!("cargo:rustc-link-lib=static=ghostty-vt");
}
