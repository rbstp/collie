use std::env;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-env-changed=GO");
    // Same resolution as tailscale-sys, so this names the Go that built the linked archive.
    let go = env::var("GO").unwrap_or_else(|_| "go".into());
    let go_version = output(
        Command::new(go)
            .env("GOTOOLCHAIN", "local")
            .args(["env", "GOVERSION"]),
    );
    let rustc_version = output(Command::new(env::var("RUSTC").unwrap()).arg("-V"));
    println!("cargo:rustc-env=COLLIE_GO_VERSION={go_version}");
    println!("cargo:rustc-env=COLLIE_RUSTC_VERSION={rustc_version}");
    println!(
        "cargo:rustc-env=COLLIE_TARGET={}",
        env::var("TARGET").unwrap()
    );
}

fn output(cmd: &mut Command) -> String {
    let out = cmd.output().unwrap_or_else(|e| panic!("{cmd:?}: {e}"));
    assert!(out.status.success(), "{cmd:?} failed");
    String::from_utf8(out.stdout).unwrap().trim().to_owned()
}
