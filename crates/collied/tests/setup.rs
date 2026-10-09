use std::process::{Command, Stdio};

#[test]
fn setup_refuses_without_a_terminal_before_touching_anything() {
    let home = tempfile::tempdir().unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_collied"))
        .arg("setup")
        .env("HOME", home.path())
        .env_remove("COLLIE_TS_AUTHKEY")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("collied setup is interactive"), "{stderr}");
    assert!(std::fs::read_dir(home.path()).unwrap().next().is_none());
}
