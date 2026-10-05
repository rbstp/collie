use protocol::KeyPin;
use tailnet::WhoIsNode;

use crate::peers::Store;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    Full { label: String, key: KeyPin },
    PairingOnly { window: u64 },
    Reject(&'static str),
}

/// Runs before any byte of the connection is read. `owner` is the configured owner,
/// falling back to the one recorded at first pairing. A pairing-only session is bound to
/// the window that admitted it.
pub fn decide(
    node: &WhoIsNode,
    owner: Option<i64>,
    store: &Store,
    pairing_window: Option<u64>,
) -> Decision {
    if node.tags.as_ref().is_some_and(|t| !t.is_empty()) {
        return Decision::Reject("tagged node");
    }
    if node.sharer != 0 {
        return Decision::Reject("shared-in node");
    }
    if let Some(owner) = owner.or(store.owner_user_id)
        && !node.is_owned_by(owner)
    {
        return Decision::Reject("not the owner");
    }
    // A phone paired before mutual TLS has no key to pin: it pairs again, through a window.
    if let Some(peer) = store.get(&node.stable_id) {
        if peer.user_id != node.user {
            return Decision::Reject("paired node changed user");
        }
        if let Some(key) = &peer.tls_key {
            return Decision::Full {
                label: peer.label.clone(),
                key: key.clone(),
            };
        }
    }
    match pairing_window {
        Some(window) => Decision::PairingOnly { window },
        None => Decision::Reject("not paired"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::peers::Peer;

    fn node(id: &str, user: i64, sharer: i64, tags: Option<Vec<&str>>) -> WhoIsNode {
        WhoIsNode {
            stable_id: id.into(),
            name: format!("{id}.tailnet.ts.net."),
            user,
            sharer,
            tags: tags.map(|t| t.into_iter().map(Into::into).collect()),
        }
    }

    fn store() -> Store {
        let mut s = Store::default();
        s.add(Peer {
            stable_id: "nPHONE".into(),
            user_id: 7,
            login: "me@example.com".into(),
            label: "phone".into(),
            paired_at: 1,
            tls_key: Some(key()),
        })
        .unwrap();
        s.add(Peer {
            stable_id: "nOLD".into(),
            user_id: 7,
            login: "me@example.com".into(),
            label: "old phone".into(),
            paired_at: 1,
            tls_key: None,
        })
        .unwrap();
        s
    }

    fn key() -> KeyPin {
        KeyPin::new("K".repeat(43)).unwrap()
    }

    #[test]
    fn paired_owner_gets_full_session() {
        let d = decide(&node("nPHONE", 7, 0, None), None, &store(), None);
        assert_eq!(
            d,
            Decision::Full {
                label: "phone".into(),
                key: key(),
            }
        );
        let d = decide(
            &node("nPHONE", 7, 0, Some(vec![])),
            Some(7),
            &store(),
            Some(1),
        );
        assert!(matches!(d, Decision::Full { .. }));
    }

    #[test]
    fn tagged_and_shared_are_always_rejected() {
        for open in [None, Some(1)] {
            for s in [Store::default(), store()] {
                let tagged = node("nPHONE", 7, 0, Some(vec!["tag:collie-phone"]));
                assert_eq!(
                    decide(&tagged, None, &s, open),
                    Decision::Reject("tagged node")
                );
                let shared = node("nPHONE", 7, 9, None);
                assert_eq!(
                    decide(&shared, Some(7), &s, open),
                    Decision::Reject("shared-in node")
                );
            }
        }
    }

    #[test]
    fn other_users_are_rejected_once_owner_is_known() {
        let stranger = node("nOTHER", 8, 0, None);
        assert_eq!(
            decide(&stranger, None, &store(), Some(1)),
            Decision::Reject("not the owner")
        );
        assert_eq!(
            decide(&stranger, Some(7), &Store::default(), Some(1)),
            Decision::Reject("not the owner")
        );
        assert_eq!(
            decide(&node("nPHONE", 8, 0, None), Some(8), &store(), Some(1)),
            Decision::Reject("paired node changed user")
        );
    }

    #[test]
    fn a_phone_paired_before_mutual_tls_pairs_again() {
        let old = node("nOLD", 7, 0, None);
        assert_eq!(
            decide(&old, None, &store(), None),
            Decision::Reject("not paired")
        );
        assert_eq!(
            decide(&old, None, &store(), Some(2)),
            Decision::PairingOnly { window: 2 }
        );
    }

    #[test]
    fn unpaired_needs_an_open_window() {
        let fresh = node("nNEW", 7, 0, None);
        assert_eq!(
            decide(&fresh, None, &store(), None),
            Decision::Reject("not paired")
        );
        assert_eq!(
            decide(&fresh, None, &store(), Some(3)),
            Decision::PairingOnly { window: 3 }
        );
        let first = node("nFIRST", 42, 0, None);
        assert_eq!(
            decide(&first, None, &Store::default(), Some(4)),
            Decision::PairingOnly { window: 4 }
        );
        assert_eq!(
            decide(&first, None, &Store::default(), None),
            Decision::Reject("not paired")
        );
    }
}
