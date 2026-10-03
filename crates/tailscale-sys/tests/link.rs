use tailscale_sys::*;

#[test]
fn links_and_allocates_a_server() {
    unsafe {
        let sd = tailscale_new();
        assert!(sd > 0);
        assert_eq!(tailscale_close(sd), 0);
    }
}
