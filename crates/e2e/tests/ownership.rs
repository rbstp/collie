#[allow(dead_code)]
mod common;

use std::net::SocketAddr;
use std::path::Path;

use sha2::{Digest, Sha256};
use std::time::Duration;

use collie_core::CoreError;
use collied::control::{Client, Reply, Request};
use collied::peers::{self, Store};
use collied::server;
use common::*;
use tokio::io::AsyncReadExt;

#[test]
fn embedded_owner_is_bound_before_listening_and_pairing() {
    if !in_child("embedded_owner_is_bound_before_listening_and_pairing") {
        return;
    }
    let root = TempDir::new("e2e-owner");
    let net = Net::start(&root.0);
    let me = wait_ready(&net.mac, 0).self_node.unwrap();
    assert!(me.user_id > 0);
    assert!(me.tags.iter().flatten().next().is_none());
    let rt = runtime();
    let core = phone(&root.0, "phone", &net);
    rt.block_on(core.node_start(Some(net.key.clone()))).unwrap();
    wait_ready(&net.mac, 1);
    wait_phone(&rt, &core);
    rt.block_on(async {
        let herdr = root.0.join("herdr.sock");
        mock_herdr(&herdr, |_, _| None);
        for owner in [0, -1, me.user_id + 1] {
            let dir = root.0.join(format!("configured-{owner}"));
            let mut cfg = server_config(&dir, "e2e");
            cfg.owner_user_id = Some(owner);
            let error = server::start(net.mac.clone(), cfg, herdr.clone())
                .await
                .err()
                .expect("owner conflict accepted");
            assert!(error.to_string().contains("owner"));
            assert!(!dir.join("control.sock").exists());
        }
        let dir = root.0.join("stored-owner");
        collied::ensure_private_dir(&dir).unwrap();
        peers::save(
            &dir.join("peers.json"),
            &Store {
                owner_user_id: Some(me.user_id + 1),
                peers: vec![],
            },
        )
        .unwrap();
        assert!(
            server::start(net.mac.clone(), server_config(&dir, "e2e"), herdr.clone())
                .await
                .is_err()
        );
        assert!(!dir.join("control.sock").exists());
        let tagged = start_node(
            &root.0,
            "old-tagged-collie",
            &net.key,
            &net.url,
            &["tag:collie-mac"],
        );
        wait_ready(&tagged, 0);
        let dir = root.0.join("tagged-owner");
        assert!(
            server::start(tagged, server_config(&dir, "e2e"), herdr.clone())
                .await
                .is_err()
        );
        assert!(!dir.join("control.sock").exists());

        let dir = root.0.join("valid-owner");
        let handle = server::start(net.mac.clone(), server_config(&dir, "e2e"), herdr)
            .await
            .unwrap();
        let (machine, _) = pair(&handle.control_path(), &core, LABEL).await;
        assert_eq!(machine.owner_user_id, me.user_id);
        connected_flock(&core, &machine.id).await;
        core.reconnect(machine.id.clone()).unwrap();
        connected_flock(&core, &machine.id).await;
        assert_udp_on_loopback();
        assert!(kernel_tcp_listeners().is_empty());
        handle.shutdown().await;
    });
}

#[test]
fn allow_all_tailnet_cannot_pair_a_foreign_user_even_first() {
    if !in_child("allow_all_tailnet_cannot_pair_a_foreign_user_even_first") {
        return;
    }
    let root = TempDir::new("e2e-foreign");
    let net = Net::with_control_args(
        &root.0,
        format!("test-authkey-owner-{}", std::process::id()),
        &[],
    );
    let me = wait_ready(&net.mac, 0).self_node.unwrap();
    let foreign = start_node(&root.0, "foreign-phone", &net.key, &net.url, &[]);
    let foreign_me = wait_ready(&foreign, 1).self_node.unwrap();
    assert_ne!(me.user_id, foreign_me.user_id);
    let ip = wait_ready(&net.mac, 1)
        .self_node
        .unwrap()
        .tailscale_ips
        .unwrap()
        .into_iter()
        .find(|ip| ip.is_ipv4())
        .unwrap();
    let rt = runtime();
    let core = phone(&root.0, "other-user-core", &net);
    rt.block_on(core.node_start(Some(net.key.clone()))).unwrap();
    wait_phone(&rt, &core);
    rt.block_on(async {
        let herdr = root.0.join("herdr.sock");
        mock_herdr(&herdr, |_, _| None);
        let dir = root.0.join("collied");
        let handle = server::start(net.mac.clone(), server_config(&dir, "e2e"), herdr)
            .await
            .unwrap();
        let mut local = Client::connect(&handle.control_path()).await.unwrap();
        let Reply::Invite { uri, .. } = local.call(&Request::Pair).await.unwrap() else {
            panic!("no invite")
        };
        let address = SocketAddr::new(ip, PORT).to_string();
        let stream = foreign
            .dial_timeout("tcp", &address, Duration::from_secs(10))
            .unwrap();
        stream.set_nonblocking(true).unwrap();
        let mut stream = tokio::net::UnixStream::from_std(stream).unwrap();
        let mut reply = vec![];
        tokio::time::timeout(Duration::from_secs(10), stream.read_to_end(&mut reply))
            .await
            .unwrap()
            .unwrap();
        assert!(reply.starts_with(b"HTTP/1.1 403"));
        assert!(
            tokio::time::timeout(Duration::from_millis(200), local.recv())
                .await
                .is_err()
        );
        assert!(!dir.join("peers.json").exists());
        assert!(matches!(
            core.pair(uri, LABEL.into()).await,
            Err(CoreError::PinViolation { .. })
        ));
        assert!(core.machines().is_empty());
        assert!(!dir.join("peers.json").exists());
        handle.shutdown().await;
    });
}

#[test]
fn restrictive_network_policy_still_blocks_user_owned_nodes() {
    if !in_child("restrictive_network_policy_still_blocks_user_owned_nodes") {
        return;
    }
    let root = TempDir::new("e2e-policy");
    let net = Net::with_control_args(
        &root.0,
        format!("test-authkey-deny-{}", std::process::id()),
        &["-same-user", "-deny-peers"],
    );
    let me = wait_ready(&net.mac, 0).self_node.unwrap();
    let phone = start_node(&root.0, "phone", &net.key, &net.url, &[]);
    let phone_me = wait_ready(&phone, 1).self_node.unwrap();
    assert_eq!(me.user_id, phone_me.user_id);
    let ip = wait_ready(&net.mac, 1)
        .self_node
        .unwrap()
        .tailscale_ips
        .unwrap()
        .into_iter()
        .find(|ip| ip.is_ipv4())
        .unwrap();
    let rt = runtime();
    let _entered = rt.enter();
    let _listener = net.mac.listen("tcp", &format!(":{PORT}")).unwrap();
    assert!(
        phone
            .dial_timeout(
                "tcp",
                &SocketAddr::new(ip, PORT).to_string(),
                Duration::from_secs(2)
            )
            .is_err()
    );
}

#[test]
fn legacy_tagged_state_is_preserved_and_refused_on_reopen() {
    if !in_child("legacy_tagged_state_is_preserved_and_refused_on_reopen") {
        return;
    }
    let root = TempDir::new("e2e-old-tags");
    let net = Net::start(&root.0);
    let node = start_node(&root.0, "legacy", &net.key, &net.url, &["tag:collie-mac"]);
    let before = wait_ready(&node, 0).self_node.unwrap();
    drop(node);
    let state = root.0.join("legacy/tailscaled.state");
    let retained = retained_identity(&state);
    let node = start_node(&root.0, "legacy", &net.key, &net.url, &[]);
    let status = wait_ready(&node, 0);
    let after = status.self_node.as_ref().unwrap();
    assert_eq!(before.stable_id, after.stable_id);
    assert_eq!(before.user_id, after.user_id);
    assert_eq!(before.tags, after.tags);
    assert!(status.auth_url.is_empty());
    assert!(node.owned_user(&status).is_err());
    let rt = runtime();
    let dir = root.0.join("refused-server");
    let error = rt
        .block_on(server::start(
            node,
            server_config(&dir, "e2e"),
            root.0.join("herdr.sock"),
        ))
        .err()
        .expect("legacy tagged server accepted");
    assert!(error.to_string().contains("untagged"));
    assert!(!dir.join("control.sock").exists());
    assert_eq!(retained, retained_identity(&state));
}

fn retained_identity(path: &Path) -> [u8; 32] {
    let state: serde_json::Map<String, serde_json::Value> =
        serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    let identity: std::collections::BTreeMap<_, _> = state
        .into_iter()
        .filter(|(key, _)| key == "_machinekey" || key.starts_with("profile-"))
        .collect();
    assert!(!identity.is_empty());
    Sha256::digest(serde_json::to_vec(&identity).unwrap()).into()
}
