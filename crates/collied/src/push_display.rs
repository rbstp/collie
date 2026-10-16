use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chacha20poly1305::aead::{Aead, Payload};
use chacha20poly1305::{ChaCha20Poly1305, KeyInit};
use protocol::NotificationKey;
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

pub const MAX_ENCODED: usize = 3800;
pub const MAX_ACTIVITY_ATTRIBUTES: usize = 1536;
pub const MAX_ACTIVITY_STATE: usize = 2400;
pub const MAX_ACTIVITY_TOTAL: usize = 4000;
const DOMAIN: &[u8] = b"collie.push.v1\0";

pub fn opaque(value: &str) -> String {
    URL_SAFE_NO_PAD.encode(&Sha256::digest(value.as_bytes())[..16])
}

pub fn fit(body: &str, max_bytes: usize) -> String {
    let escaped = |c: char| serde_json::to_string(&c).map_or(6, |s| s.len() - 2);
    if body.chars().map(escaped).sum::<usize>() <= max_bytes {
        return body.to_owned();
    }
    let mut used = escaped('…');
    let mut out: String = body
        .chars()
        .take_while(|&c| {
            used += escaped(c);
            used <= max_bytes
        })
        .collect();
    out.push('…');
    out
}

pub fn seal(
    key: &NotificationKey,
    kind: u8,
    node_id: &str,
    scope: &str,
    event: [u8; 16],
    nonce: [u8; 12],
    plaintext: &[u8],
) -> Option<String> {
    if !(1..=3).contains(&kind)
        || protocol::Label::new(node_id).is_err()
        || !node_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        || node_id.len() > 64
        || scope.is_empty()
        || scope.len() > 64
        || !scope
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        return None;
    }
    let key = Zeroizing::new(URL_SAFE_NO_PAD.decode(key.as_str()).ok()?);
    let mut header = vec![1, kind, node_id.len() as u8, scope.len() as u8];
    header.extend_from_slice(node_id.as_bytes());
    header.extend_from_slice(scope.as_bytes());
    header.extend_from_slice(&event);
    let mut aad = DOMAIN.to_vec();
    aad.extend_from_slice(&header);
    aad.extend_from_slice(&Sha256::digest(&*key));
    let sealed = ChaCha20Poly1305::new_from_slice(&key)
        .ok()?
        .encrypt(
            &nonce.into(),
            Payload {
                msg: plaintext,
                aad: &aad,
            },
        )
        .ok()?;
    header.extend_from_slice(&nonce);
    header.extend_from_slice(&sealed);
    let enc = URL_SAFE_NO_PAD.encode(header);
    (enc.len() <= MAX_ENCODED).then_some(enc)
}

pub fn seal_fresh(
    key: &NotificationKey,
    kind: u8,
    node_id: &str,
    scope: &str,
    plaintext: &[u8],
) -> Option<(String, String)> {
    let mut event = [0u8; 16];
    let mut nonce = [0u8; 12];
    getrandom::fill(&mut event).ok()?;
    getrandom::fill(&mut nonce).ok()?;
    Some((
        URL_SAFE_NO_PAD.encode(event),
        seal(key, kind, node_id, scope, event, nonce, plaintext)?,
    ))
}

#[cfg(test)]
pub fn open(
    key: &NotificationKey,
    enc: &str,
    kind: u8,
    node: &str,
    scope: &str,
    event_id: &str,
) -> Option<serde_json::Value> {
    if enc.len() > MAX_ENCODED {
        return None;
    }
    let bytes = URL_SAFE_NO_PAD.decode(enc).ok()?;
    if URL_SAFE_NO_PAD.encode(&bytes) != enc
        || bytes.len() < 48
        || bytes[0] != 1
        || bytes[1] != kind
    {
        return None;
    }
    let header_len = 4 + usize::from(bytes[2]) + usize::from(bytes[3]) + 16;
    let header = bytes.get(..header_len)?;
    let node_end = 4 + usize::from(bytes[2]);
    let scope_end = node_end + usize::from(bytes[3]);
    if bytes.get(4..node_end)? != node.as_bytes()
        || bytes.get(node_end..scope_end)? != scope.as_bytes()
        || URL_SAFE_NO_PAD.encode(bytes.get(scope_end..header_len)?) != event_id
    {
        return None;
    }
    let key = Zeroizing::new(URL_SAFE_NO_PAD.decode(key.as_str()).ok()?);
    let mut aad = DOMAIN.to_vec();
    aad.extend_from_slice(header);
    aad.extend_from_slice(&Sha256::digest(&*key));
    let nonce: [u8; 12] = bytes.get(header_len..header_len + 12)?.try_into().ok()?;
    let plaintext = Zeroizing::new(
        ChaCha20Poly1305::new_from_slice(&key)
            .ok()?
            .decrypt(
                &nonce.into(),
                Payload {
                    msg: bytes.get(header_len + 12..)?,
                    aad: &aad,
                },
            )
            .ok()?,
    );
    serde_json::from_slice(&plaintext).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    #[test]
    fn shared_activity_vectors_fit_the_combined_budget_with_approval_context() {
        let fixture: Value = serde_json::from_str(include_str!(
            "../../../docs/protocol/push-display-vectors.json"
        ))
        .unwrap();
        assert_eq!(
            fixture["activity_limits"],
            json!({"attributes_bytes":MAX_ACTIVITY_ATTRIBUTES,"state_bytes":MAX_ACTIVITY_STATE,"total_bytes":MAX_ACTIVITY_TOTAL,"context_bytes":crate::activity::MAX_CONTEXT_BYTES})
        );
        let key = crate::push::tests::key();
        for v in fixture["activity_limit_vectors"].as_array().unwrap() {
            let attrs = &v["attributes"];
            let state = &v["content_state"];
            let attrs_bytes = attrs.to_string().len();
            let state_bytes = state.to_string().len();
            assert_eq!(json!(attrs_bytes), v["attributes_bytes"]);
            assert_eq!(json!(state_bytes), v["state_bytes"]);
            assert_eq!(json!(attrs_bytes + state_bytes), v["combined_bytes"]);
            assert!(attrs_bytes <= MAX_ACTIVITY_ATTRIBUTES);
            assert!(state_bytes <= MAX_ACTIVITY_STATE);
            assert!(MAX_ACTIVITY_ATTRIBUTES + state_bytes <= MAX_ACTIVITY_TOTAL);
            let node = attrs["nodeId"].as_str().unwrap();
            let scope = attrs["bindingId"].as_str().unwrap();
            let event = "AAECAwQFBgcICQoLDA0ODw";
            let plain = open(&key, state["enc"].as_str().unwrap(), 2, node, scope, event).unwrap();
            assert_eq!(plain["state"], v["private_state"]);
            let attrs_display =
                open(&key, attrs["enc"].as_str().unwrap(), 3, node, scope, event).unwrap();
            assert_eq!(attrs_display["title"], plain["state"]["title"]);
            assert_eq!(attrs_display["machineLabel"], plain["state"]["workspace"]);
            let command = crate::push::tests::open(
                &key,
                plain["state"]["enc"].as_str().unwrap(),
                plain["state"]["approvalId"].as_str().unwrap(),
            )
            .unwrap();
            assert_eq!(command["body"], v["context"]);
            assert!(
                serde_json::to_string(v["context"].as_str().unwrap())
                    .unwrap()
                    .len()
                    - 2
                    <= crate::activity::MAX_CONTEXT_BYTES
            );
        }
    }

    #[test]
    fn shared_vectors_authenticate_every_binding_and_ciphertext() {
        let fixture: Value = serde_json::from_str(include_str!(
            "../../../docs/protocol/push-display-vectors.json"
        ))
        .unwrap();
        let key = crate::push::tests::key();
        for v in fixture["vectors"].as_array().unwrap() {
            let kind = v["kind"].as_u64().unwrap() as u8;
            let node = v["node_id"].as_str().unwrap();
            let scope = v["scope"].as_str().unwrap();
            let event = v["event_id"].as_str().unwrap();
            let plain = v["plaintext"].as_str().unwrap();
            let enc = seal(
                &key,
                kind,
                node,
                scope,
                std::array::from_fn(|i| i as u8),
                std::array::from_fn(|i| 0xa0 + i as u8),
                plain.as_bytes(),
            )
            .unwrap();
            assert_eq!(v["enc"], enc);
            assert_eq!(
                open(&key, &enc, kind, node, scope, event),
                Some(serde_json::from_str(plain).unwrap())
            );
            for (other_kind, other_node, other_scope, other_event) in [
                (kind % 3 + 1, node, scope, event),
                (kind, "nOther", scope, event),
                (kind, node, "another-scope", event),
                (kind, node, scope, "another-event"),
            ] {
                assert!(
                    open(&key, &enc, other_kind, other_node, other_scope, other_event).is_none()
                );
            }
            let wrong_key = NotificationKey::new(URL_SAFE_NO_PAD.encode([99u8; 32])).unwrap();
            assert!(open(&wrong_key, &enc, kind, node, scope, event).is_none());
            let mut bytes = URL_SAFE_NO_PAD.decode(&enc).unwrap();
            let end = bytes.len() - 1;
            bytes[end] ^= 1;
            assert!(
                open(
                    &key,
                    &URL_SAFE_NO_PAD.encode(bytes),
                    kind,
                    node,
                    scope,
                    event
                )
                .is_none()
            );
            assert!(open(&key, &format!("{enc}="), kind, node, scope, event).is_none());
            let mut nonce = [0u8; 12];
            getrandom::fill(&mut nonce).unwrap();
            assert!(seal(&key, kind, node, scope, [0; 16], nonce, &vec![0; 4096]).is_none());
        }
        assert_eq!(
            fixture["vectors"][0]["payload"]["aps"]["alert"],
            json!({"title":"Collie","body":"An agent needs attention"})
        );
    }
}
