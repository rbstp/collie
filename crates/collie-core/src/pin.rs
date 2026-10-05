use std::net::IpAddr;

use tailnet::{PeerStatus, Status, WhoIsNode};

use crate::store::MachineKind;

#[cfg(test)]
pub const MAC_TAG: &str = "tag:collie-mac";

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PinError {
    #[error("{host} is not in this tailnet's netmap")]
    MissingPeer { host: String },
    #[error("{host} is node {found}, not the paired node {expected}")]
    NodeIdMismatch {
        host: String,
        expected: String,
        found: String,
    },
    #[error("{host} is not tagged {expected}")]
    Untagged { host: String, expected: String },
    #[error("{host} has no tailnet address")]
    NoAddress { host: String },
    #[error("{host} is shared in from another tailnet")]
    SharedIn { host: String },
    #[error("{host} did not present the key it was paired with")]
    KeyMismatch { host: String },
}

impl PinError {
    /// A pin violation means the name now points at another node, or the machine lost
    /// its tag: retrying cannot fix it and must not be automatic.
    pub fn is_violation(&self) -> bool {
        matches!(
            self,
            Self::NodeIdMismatch { .. }
                | Self::Untagged { .. }
                | Self::SharedIn { .. }
                | Self::KeyMismatch { .. }
        )
    }
}

/// The IP is taken from the netmap entry of the pinned node itself, so the
/// connection never depends on a DNS answer. `kind` is None only when pairing, where
/// either collie tag is accepted and the one found is returned to be pinned.
pub fn resolve(
    status: &Status,
    host: &str,
    node_id: &str,
    kind: Option<MachineKind>,
) -> Result<(IpAddr, MachineKind), PinError> {
    let named: Vec<&PeerStatus> = status
        .peer
        .iter()
        .flat_map(|peers| peers.values())
        .filter(|p| p.dns_name.trim_end_matches('.').eq_ignore_ascii_case(host))
        .collect();
    let peer = match named.iter().find(|p| p.stable_id == node_id) {
        Some(peer) => *peer,
        None => {
            return Err(match named.first() {
                None => PinError::MissingPeer { host: host.into() },
                Some(other) => PinError::NodeIdMismatch {
                    host: host.into(),
                    expected: node_id.into(),
                    found: other.stable_id.clone(),
                },
            });
        }
    };
    let pinned;
    let accepted: &[MachineKind] = match kind {
        Some(k) => {
            pinned = [k];
            &pinned
        }
        None => &MachineKind::ALL,
    };
    let found = accepted
        .iter()
        .copied()
        .find(|k| peer.tags.iter().flatten().any(|t| t == k.tag()))
        .ok_or_else(|| PinError::Untagged {
            host: host.into(),
            expected: accepted
                .iter()
                .map(|k| k.tag())
                .collect::<Vec<_>>()
                .join(" or "),
        })?;
    let ips = peer.tailscale_ips.as_deref().unwrap_or_default();
    let ip = ips
        .iter()
        .find(|ip| ip.is_ipv4())
        .or_else(|| ips.first())
        .copied()
        .ok_or(PinError::NoAddress { host: host.into() })?;
    Ok((ip, found))
}

/// Applied to whois of the resolved IP and to the node ID the machine reports in hello, so
/// a shared-in node or a stale netmap entry cannot stand in for the pinned machine.
pub fn verify_node(found: &str, host: &str, node_id: &str) -> Result<(), PinError> {
    if found != node_id {
        return Err(PinError::NodeIdMismatch {
            host: host.into(),
            expected: node_id.into(),
            found: found.into(),
        });
    }
    Ok(())
}

pub fn verify_whois(who: &WhoIsNode, host: &str, node_id: &str) -> Result<(), PinError> {
    verify_node(&who.stable_id, host, node_id)?;
    if who.sharer != 0 {
        return Err(PinError::SharedIn { host: host.into() });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOST: &str = "mac.tail1234.ts.net";

    fn status(peers: &[(&str, &str, &[&str], &[&str])]) -> Status {
        let peers: serde_json::Map<String, serde_json::Value> = peers
            .iter()
            .map(|(id, dns, tags, ips)| {
                (
                    format!("nodekey:{id}"),
                    serde_json::json!({
                        "ID": id,
                        "DNSName": dns,
                        "Tags": tags,
                        "TailscaleIPs": ips,
                    }),
                )
            })
            .collect();
        serde_json::from_value(serde_json::json!({
            "BackendState": "Running",
            "Self": {"ID": "nPHONE", "DNSName": "phone.tail1234.ts.net."},
            "Peer": peers,
        }))
        .unwrap()
    }

    #[test]
    fn pinned_tagged_mac_resolves_to_ipv4() {
        let st = status(&[
            ("nOTHER", "other.tail1234.ts.net.", &[], &["100.64.0.9"]),
            (
                "nMAC",
                "mac.tail1234.ts.net.",
                &[MAC_TAG],
                &["fd7a:115c:a1e0::1", "100.64.0.1"],
            ),
        ]);
        assert_eq!(
            resolve(&st, HOST, "nMAC", Some(MachineKind::Mac)),
            Ok(("100.64.0.1".parse().unwrap(), MachineKind::Mac))
        );
        assert_eq!(
            resolve(&st, "MAC.tail1234.ts.net", "nMAC", Some(MachineKind::Mac)),
            Ok(("100.64.0.1".parse().unwrap(), MachineKind::Mac))
        );
    }

    #[test]
    fn wrong_stable_id_is_a_violation() {
        let st = status(&[(
            "nIMPOSTOR",
            "mac.tail1234.ts.net.",
            &[MAC_TAG],
            &["100.64.0.7"],
        )]);
        let err = resolve(&st, HOST, "nMAC", Some(MachineKind::Mac)).unwrap_err();
        assert_eq!(
            err,
            PinError::NodeIdMismatch {
                host: HOST.into(),
                expected: "nMAC".into(),
                found: "nIMPOSTOR".into()
            }
        );
        assert!(err.is_violation());
    }

    #[test]
    fn untagged_mac_is_a_violation() {
        let st = status(&[("nMAC", "mac.tail1234.ts.net.", &[], &["100.64.0.1"])]);
        let err = resolve(&st, HOST, "nMAC", Some(MachineKind::Mac)).unwrap_err();
        assert_eq!(
            err,
            PinError::Untagged {
                host: HOST.into(),
                expected: MAC_TAG.into()
            }
        );
        assert!(err.is_violation());
        let st = status(&[(
            "nMAC",
            "mac.tail1234.ts.net.",
            &["tag:collie-macx"],
            &["100.64.0.1"],
        )]);
        assert!(
            resolve(&st, HOST, "nMAC", Some(MachineKind::Mac))
                .unwrap_err()
                .is_violation()
        );
    }

    #[test]
    fn missing_peer_is_transient() {
        let st = status(&[("nMAC", "mac2.tail1234.ts.net.", &[MAC_TAG], &["100.64.0.1"])]);
        let err = resolve(&st, HOST, "nMAC", Some(MachineKind::Mac)).unwrap_err();
        assert_eq!(err, PinError::MissingPeer { host: HOST.into() });
        assert!(!err.is_violation());
        let empty: Status =
            serde_json::from_str(r#"{"BackendState":"Running","Peer":null}"#).unwrap();
        assert!(matches!(
            resolve(&empty, HOST, "nMAC", Some(MachineKind::Mac)),
            Err(PinError::MissingPeer { .. })
        ));
    }

    #[test]
    fn stable_id_elsewhere_in_netmap_does_not_count() {
        let st = status(&[
            (
                "nMAC",
                "renamed.tail1234.ts.net.",
                &[MAC_TAG],
                &["100.64.0.1"],
            ),
            (
                "nIMPOSTOR",
                "mac.tail1234.ts.net.",
                &[MAC_TAG],
                &["100.64.0.7"],
            ),
        ]);
        assert!(matches!(
            resolve(&st, HOST, "nMAC", Some(MachineKind::Mac)),
            Err(PinError::NodeIdMismatch { .. })
        ));
    }

    #[test]
    fn no_address() {
        let st = status(&[("nMAC", "mac.tail1234.ts.net.", &[MAC_TAG], &[])]);
        assert_eq!(
            resolve(&st, HOST, "nMAC", Some(MachineKind::Mac)),
            Err(PinError::NoAddress { host: HOST.into() })
        );
    }

    #[test]
    fn pairing_accepts_either_tag_and_reports_which() {
        let tagged =
            |tags: &[&str]| status(&[("nM", "mac.tail1234.ts.net.", tags, &["100.64.0.1"])]);
        let ip: IpAddr = "100.64.0.1".parse().unwrap();
        for kind in MachineKind::ALL {
            let st = tagged(&[kind.tag()]);
            assert_eq!(resolve(&st, HOST, "nM", None), Ok((ip, kind)));
            assert_eq!(resolve(&st, HOST, "nM", Some(kind)), Ok((ip, kind)));
        }
        for tags in [
            &[][..],
            &["tag:collie-linuxx"],
            &["tag:collie-phone"],
            &["tag:collie"],
        ] {
            let err = resolve(&tagged(tags), HOST, "nM", None).unwrap_err();
            assert!(err.is_violation(), "{tags:?}");
            assert_eq!(
                err.to_string(),
                format!("{HOST} is not tagged tag:collie-mac or tag:collie-linux")
            );
        }
    }

    #[test]
    fn a_paired_machine_must_keep_its_kind() {
        let linux = status(&[(
            "nM",
            "mac.tail1234.ts.net.",
            &["tag:collie-linux"],
            &["100.64.0.1"],
        )]);
        let err = resolve(&linux, HOST, "nM", Some(MachineKind::Mac)).unwrap_err();
        assert_eq!(
            err,
            PinError::Untagged {
                host: HOST.into(),
                expected: MAC_TAG.into()
            }
        );
        assert!(err.is_violation());
        let mac = status(&[("nM", "mac.tail1234.ts.net.", &[MAC_TAG], &["100.64.0.1"])]);
        assert!(
            resolve(&mac, HOST, "nM", Some(MachineKind::Linux))
                .unwrap_err()
                .is_violation()
        );
    }

    fn whois(id: &str, sharer: i64) -> WhoIsNode {
        serde_json::from_value(serde_json::json!({
            "StableID": id,
            "Name": "mac.tail1234.ts.net.",
            "User": 7,
            "Sharer": sharer,
            "Tags": [MAC_TAG],
        }))
        .unwrap()
    }

    #[test]
    fn whois_must_match_pin_and_not_be_shared_in() {
        assert_eq!(verify_whois(&whois("nMAC", 0), HOST, "nMAC"), Ok(()));
        let err = verify_whois(&whois("nOTHER", 0), HOST, "nMAC").unwrap_err();
        assert!(matches!(err, PinError::NodeIdMismatch { .. }));
        let err = verify_whois(&whois("nMAC", 42), HOST, "nMAC").unwrap_err();
        assert_eq!(err, PinError::SharedIn { host: HOST.into() });
        assert!(err.is_violation());
        let without_sharer: WhoIsNode = serde_json::from_value(serde_json::json!({
            "StableID": "nMAC", "Name": "mac.", "User": 7
        }))
        .unwrap();
        assert_eq!(verify_whois(&without_sharer, HOST, "nMAC"), Ok(()));
    }

    #[test]
    fn hello_node_id_must_match_pin() {
        assert_eq!(verify_node("nMAC", HOST, "nMAC"), Ok(()));
        assert!(
            verify_node("nOTHER", HOST, "nMAC")
                .unwrap_err()
                .is_violation()
        );
        assert!(verify_node("", HOST, "nMAC").unwrap_err().is_violation());
    }
}
