use std::net::IpAddr;

use tailnet::{PeerStatus, Status, WhoIsNode};

use crate::store::MachineKind;

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
    #[error(
        "{host} must be an untagged, unshared node owned by this Collie sign-in; sign in with the same personal account and pair again"
    )]
    Ownership { host: String },
    #[error(
        "Collie's embedded Tailscale sign-in does not match the paired owner; sign in with the same personal account and pair again"
    )]
    SelfOwnership,
    #[error("this phone's Tailscale identity is not known yet")]
    SelfUnknown,
    #[error("{host} has no tailnet address")]
    NoAddress { host: String },
    #[error("{host} reports an unsupported platform: {os}")]
    Platform { host: String, os: String },
    #[error("{host} did not present the key it was paired with")]
    KeyMismatch { host: String },
}

impl PinError {
    pub fn is_violation(&self) -> bool {
        matches!(
            self,
            Self::NodeIdMismatch { .. }
                | Self::Ownership { .. }
                | Self::SelfOwnership
                | Self::KeyMismatch { .. }
                | Self::Platform { .. }
        )
    }
}

pub fn self_user(status: &Status, owner: Option<i64>) -> Result<i64, PinError> {
    let me = status.self_node.as_ref().ok_or(PinError::SelfUnknown)?;
    if me.tags.as_ref().is_some_and(|tags| !tags.is_empty()) {
        return Err(PinError::SelfOwnership);
    }
    if me.user_id <= 0 || me.stable_id.is_empty() {
        return Err(PinError::SelfUnknown);
    }
    if owner.is_some_and(|owner| owner <= 0 || owner != me.user_id) {
        return Err(PinError::SelfOwnership);
    }
    Ok(me.user_id)
}

/// The address comes from the pinned node's netmap entry, never a DNS answer.
pub fn resolve(
    status: &Status,
    host: &str,
    node_id: &str,
    kind: Option<MachineKind>,
    owner: Option<i64>,
) -> Result<(IpAddr, MachineKind), PinError> {
    let owner = self_user(status, owner)?;
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
    if peer.user_id != owner || peer.tags.as_ref().is_some_and(|tags| !tags.is_empty()) {
        return Err(PinError::Ownership { host: host.into() });
    }
    let kind = match kind {
        Some(kind) => kind,
        None => match peer.os.to_ascii_lowercase().as_str() {
            "macos" => MachineKind::Mac,
            "linux" => MachineKind::Linux,
            _ => {
                return Err(PinError::Platform {
                    host: host.into(),
                    os: peer.os.clone(),
                });
            }
        },
    };
    let ips = peer.tailscale_ips.as_deref().unwrap_or_default();
    let ip = ips
        .iter()
        .find(|ip| ip.is_ipv4())
        .or_else(|| ips.first())
        .copied()
        .ok_or(PinError::NoAddress { host: host.into() })?;
    Ok((ip, kind))
}

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

pub fn verify_whois(
    who: &WhoIsNode,
    host: &str,
    node_id: &str,
    owner: i64,
) -> Result<(), PinError> {
    verify_node(&who.stable_id, host, node_id)?;
    if !who.is_owned_by(owner) {
        return Err(PinError::Ownership { host: host.into() });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const HOST: &str = "mac.test.ts.net";

    fn status() -> Status {
        serde_json::from_value(json!({
            "BackendState": "Running",
            "Self": {"ID": "nPHONE", "UserID": 7, "TailscaleIPs": ["100.64.0.2"]},
            "Peer": {"key": {"ID": "nMAC", "DNSName": "mac.test.ts.net.",
                "UserID": 7, "OS": "macOS", "TailscaleIPs": ["fd7a:115c:a1e0::1", "100.64.0.1"]}}
        }))
        .unwrap()
    }

    fn whois() -> WhoIsNode {
        serde_json::from_value(json!({"StableID": "nMAC", "Name": HOST, "User": 7})).unwrap()
    }

    #[test]
    fn same_user_pairing_and_reconnect_keep_exact_node_and_prefer_ipv4() {
        let st = status();
        for owner in [None, Some(7)] {
            assert_eq!(
                resolve(&st, HOST, "nMAC", None, owner),
                Ok(("100.64.0.1".parse().unwrap(), MachineKind::Mac))
            );
        }
        assert!(
            resolve(
                &st,
                "MAC.TEST.TS.NET",
                "nMAC",
                Some(MachineKind::Mac),
                Some(7)
            )
            .is_ok()
        );
        assert_eq!(verify_whois(&whois(), HOST, "nMAC", 7), Ok(()));
    }

    #[test]
    fn unknown_or_changed_phone_owner_never_upgrades_a_saved_pairing() {
        for owner in [0, -1, 8] {
            assert_eq!(
                resolve(&status(), HOST, "nMAC", Some(MachineKind::Mac), Some(owner)),
                Err(PinError::SelfOwnership)
            );
        }
        let mut st = status();
        st.self_node.as_mut().unwrap().user_id = 8;
        assert_eq!(
            resolve(&st, HOST, "nMAC", None, Some(7)),
            Err(PinError::SelfOwnership)
        );
        // Not known yet, as a node may report before it has its netmap: retried, not a stop.
        for user in [0, -1] {
            let mut st = status();
            st.self_node.as_mut().unwrap().user_id = user;
            assert_eq!(
                resolve(&st, HOST, "nMAC", None, Some(7)),
                Err(PinError::SelfUnknown)
            );
        }
        let mut st = status();
        st.self_node.as_mut().unwrap().stable_id = String::new();
        assert_eq!(
            resolve(&st, HOST, "nMAC", None, Some(7)),
            Err(PinError::SelfUnknown)
        );
        let mut st = status();
        st.self_node = None;
        assert_eq!(
            resolve(&st, HOST, "nMAC", None, None),
            Err(PinError::SelfUnknown)
        );
        assert!(!PinError::SelfUnknown.is_violation());
        for user in [7, 0] {
            let mut st = status();
            let me = st.self_node.as_mut().unwrap();
            me.tags = Some(vec!["tag:phone".into()]);
            me.user_id = user;
            assert_eq!(
                resolve(&st, HOST, "nMAC", None, None),
                Err(PinError::SelfOwnership)
            );
        }
    }

    #[test]
    fn foreign_zero_and_tagged_machine_fail_before_dial() {
        for user in [0, -1, 8] {
            let mut st = status();
            st.peer
                .as_mut()
                .unwrap()
                .values_mut()
                .next()
                .unwrap()
                .user_id = user;
            assert!(matches!(
                resolve(&st, HOST, "nMAC", None, None),
                Err(PinError::Ownership { .. })
            ));
        }
        let mut st = status();
        st.peer.as_mut().unwrap().values_mut().next().unwrap().tags =
            Some(vec!["tag:collie-mac".into()]);
        assert!(matches!(
            resolve(&st, HOST, "nMAC", Some(MachineKind::Mac), Some(7)),
            Err(PinError::Ownership { .. })
        ));
    }

    #[test]
    fn fresh_whois_cannot_replace_owner_node_or_sharing() {
        for user in [0, -1, 8] {
            let mut who = whois();
            who.user = user;
            assert!(matches!(
                verify_whois(&who, HOST, "nMAC", 7),
                Err(PinError::Ownership { .. })
            ));
        }
        let mut who = whois();
        who.sharer = 9;
        assert!(verify_whois(&who, HOST, "nMAC", 7).is_err());
        let mut who = whois();
        who.tags = Some(vec!["tag:collie-mac".into()]);
        assert!(verify_whois(&who, HOST, "nMAC", 7).is_err());
        let mut who = whois();
        who.stable_id = "nOTHER".into();
        assert!(matches!(
            verify_whois(&who, HOST, "nMAC", 7),
            Err(PinError::NodeIdMismatch { .. })
        ));
        assert!(verify_whois(&whois(), HOST, "nMAC", 0).is_err());
    }

    #[test]
    fn missing_peer_is_transient_but_reused_name_is_a_violation() {
        let mut st = status();
        st.peer = None;
        assert!(
            !resolve(&st, HOST, "nMAC", None, None)
                .unwrap_err()
                .is_violation()
        );
        let mut st = status();
        st.peer
            .as_mut()
            .unwrap()
            .values_mut()
            .next()
            .unwrap()
            .stable_id = "nOTHER".into();
        assert!(
            resolve(&st, HOST, "nMAC", None, None)
                .unwrap_err()
                .is_violation()
        );
        assert!(
            verify_node("nOTHER", HOST, "nMAC")
                .unwrap_err()
                .is_violation()
        );
        assert!(verify_node("", HOST, "nMAC").unwrap_err().is_violation());
    }

    #[test]
    fn platform_is_presentation_and_never_relaxes_ownership() {
        let mut st = status();
        let peer = st.peer.as_mut().unwrap().values_mut().next().unwrap();
        peer.os = "linux".into();
        assert_eq!(
            resolve(&st, HOST, "nMAC", None, None).unwrap().1,
            MachineKind::Linux
        );
        assert_eq!(
            resolve(&st, HOST, "nMAC", Some(MachineKind::Mac), Some(7))
                .unwrap()
                .1,
            MachineKind::Mac
        );
        st.peer
            .as_mut()
            .unwrap()
            .values_mut()
            .next()
            .unwrap()
            .os
            .clear();
        assert!(matches!(
            resolve(&st, HOST, "nMAC", None, None),
            Err(PinError::Platform { .. })
        ));
        assert!(resolve(&st, HOST, "nMAC", Some(MachineKind::Mac), Some(7)).is_ok());
        st.peer
            .as_mut()
            .unwrap()
            .values_mut()
            .next()
            .unwrap()
            .user_id = 8;
        assert!(matches!(
            resolve(&st, HOST, "nMAC", Some(MachineKind::Mac), Some(7)),
            Err(PinError::Ownership { .. })
        ));
    }

    #[test]
    fn missing_address_is_transient() {
        let mut st = status();
        st.peer
            .as_mut()
            .unwrap()
            .values_mut()
            .next()
            .unwrap()
            .tailscale_ips = None;
        assert!(matches!(
            resolve(&st, HOST, "nMAC", None, None),
            Err(PinError::NoAddress { .. })
        ));
    }
}
