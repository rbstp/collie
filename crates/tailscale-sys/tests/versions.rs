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
fn testcontrol_pins_the_tailscale_version_of_libtailscale() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let ours = fs::read_to_string(concat!(env!("OUT_DIR"), "/libtailscale/go.mod")).unwrap();
    let go_mod = fs::read_to_string(dir.join("../tailnet/testcontrol/go.mod")).unwrap();
    assert_eq!(
        tailscale_version(go_mod.lines()),
        tailscale_version(ours.lines()),
        "crates/tailnet/testcontrol and libtailscale must require the same tailscale.com"
    );
}
