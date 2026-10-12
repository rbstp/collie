use std::fs;
use std::path::Path;

fn tailscale_version<'a>(mut lines: impl Iterator<Item = &'a str>) -> &'a str {
    lines
        .find_map(|l| {
            l.trim_start_matches("require ")
                .trim()
                .strip_prefix("tailscale.com ")
        })
        .expect("a tailscale.com requirement")
}

#[test]
fn testcontrol_pins_the_tailscale_version_of_patch_0001() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let bumps: Vec<String> = fs::read_dir(dir.join("patches"))
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .filter(|n| n.starts_with("0001-"))
        .collect();
    assert_eq!(bumps.len(), 1, "{bumps:?}");
    let patch = fs::read_to_string(dir.join("patches").join(&bumps[0])).unwrap();
    let ours = tailscale_version(patch.lines().filter_map(|l| l.strip_prefix('+')));
    assert_eq!(bumps[0], format!("0001-bump-tailscale-{ours}.patch"));
    let go_mod = fs::read_to_string(dir.join("../tailnet/testcontrol/go.mod")).unwrap();
    assert_eq!(
        tailscale_version(go_mod.lines()),
        ours,
        "crates/tailnet/testcontrol and patch 0001 must require the same tailscale.com"
    );
}
