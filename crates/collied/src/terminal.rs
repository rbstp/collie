use std::collections::HashMap;
use std::time::{Duration, Instant};

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use protocol::{Nonce, Signature, TerminalId, TerminalKey};
use subtle::ConstantTimeEq;
use zeroize::Zeroizing;

pub const CHALLENGE_TTL: Duration = Duration::from_secs(60);
pub const GRANT_TTL: Duration = Duration::from_secs(300);
const MAX_GRANTS: usize = 16;

/// Challenges and grants live in memory only, per session: a new connection, a restart or a
/// revoke starts with none.
#[derive(Default)]
pub struct Grants {
    sessions: HashMap<u64, Session>,
}

#[derive(Default)]
struct Session {
    challenge: Option<Challenge>,
    grants: Vec<Grant>,
}

struct Challenge {
    terminal_id: TerminalId,
    nonce: Nonce,
    expires: Instant,
}

/// Bound to the key it was verified against, so a re-pairing that changes the key ends it.
struct Grant {
    terminal_id: TerminalId,
    key: TerminalKey,
    expires: Instant,
}

impl Grants {
    /// Replaces the session's outstanding challenge, if any.
    pub fn challenge(
        &mut self,
        session: u64,
        terminal_id: &TerminalId,
        now: Instant,
    ) -> anyhow::Result<Nonce> {
        let mut bytes = Zeroizing::new([0u8; protocol::limits::NONCE_BYTES]);
        getrandom::fill(bytes.as_mut_slice()).map_err(|e| anyhow::anyhow!("getrandom: {e}"))?;
        let nonce = Nonce::new(URL_SAFE_NO_PAD.encode(bytes.as_slice()))?;
        self.sessions.entry(session).or_default().challenge = Some(Challenge {
            terminal_id: terminal_id.clone(),
            nonce: nonce.clone(),
            expires: now + CHALLENGE_TTL,
        });
        Ok(nonce)
    }

    /// Any attempt burns the session's challenge, whatever the result.
    pub fn take_challenge(
        &mut self,
        session: u64,
        terminal_id: &TerminalId,
        given: &Nonce,
        now: Instant,
    ) -> Result<(), &'static str> {
        let Some(c) = self
            .sessions
            .get_mut(&session)
            .and_then(|s| s.challenge.take())
        else {
            return Err("no challenge was issued");
        };
        if now >= c.expires {
            return Err("challenge expired");
        }
        let same: bool = c
            .nonce
            .as_str()
            .as_bytes()
            .ct_eq(given.as_str().as_bytes())
            .into();
        if !same || c.terminal_id != *terminal_id {
            return Err("challenge does not match");
        }
        Ok(())
    }

    pub fn grant(
        &mut self,
        session: u64,
        terminal_id: &TerminalId,
        key: &TerminalKey,
        now: Instant,
        ttl: Duration,
    ) {
        let grants = &mut self.sessions.entry(session).or_default().grants;
        grants.retain(|g| now < g.expires && g.terminal_id != *terminal_id);
        if grants.len() == MAX_GRANTS {
            grants.remove(0);
        }
        grants.push(Grant {
            terminal_id: terminal_id.clone(),
            key: key.clone(),
            expires: now + ttl,
        });
    }

    pub fn granted(
        &mut self,
        session: u64,
        terminal_id: &TerminalId,
        key: &TerminalKey,
        now: Instant,
    ) -> bool {
        let Some(s) = self.sessions.get_mut(&session) else {
            return false;
        };
        s.grants.retain(|g| now < g.expires);
        s.grants
            .iter()
            .any(|g| g.terminal_id == *terminal_id && g.key == *key)
    }

    /// `terminal.lock`, the end of a session, a revoke or a re-pairing.
    pub fn end_session(&mut self, session: u64) {
        self.sessions.remove(&session);
    }

    /// An agent runs there now: a shell after it needs a new grant.
    pub fn drop_terminal(&mut self, terminal_id: &str) {
        for s in self.sessions.values_mut() {
            s.grants.retain(|g| g.terminal_id.as_str() != terminal_id);
        }
    }

    pub fn is_empty(&self) -> bool {
        self.sessions.values().all(|s| s.grants.is_empty())
    }
}

/// `signature` (DER, base64url) by `key` over the grant message for this machine,
/// terminal and challenge.
pub fn verify(
    key: &TerminalKey,
    node_id: &str,
    terminal_id: &TerminalId,
    challenge: &Nonce,
    signature: &Signature,
) -> bool {
    let (Ok(spki), Ok(der)) = (
        URL_SAFE_NO_PAD.decode(key.as_str()),
        URL_SAFE_NO_PAD.decode(signature.as_str()),
    ) else {
        return false;
    };
    let message = protocol::terminal_grant_message(node_id, terminal_id, challenge);
    collie_tls::verify(&spki, &message, &der)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tid(s: &str) -> TerminalId {
        TerminalId::new(s).unwrap()
    }

    fn key(c: char) -> TerminalKey {
        TerminalKey::new(format!("{}A", c.to_string().repeat(121))).unwrap()
    }

    #[test]
    fn a_challenge_is_single_use_and_bound() {
        let mut g = Grants::default();
        let now = Instant::now();
        let t1 = tid("term_1");
        let n = g.challenge(1, &t1, now).unwrap();
        assert_eq!(g.take_challenge(1, &t1, &n, now), Ok(()));
        assert_eq!(
            g.take_challenge(1, &t1, &n, now),
            Err("no challenge was issued"),
            "burned"
        );

        let n = g.challenge(1, &t1, now).unwrap();
        assert_eq!(
            g.take_challenge(1, &tid("term_2"), &n, now),
            Err("challenge does not match"),
            "another terminal"
        );
        assert!(
            g.take_challenge(1, &t1, &n, now).is_err(),
            "burned by a failure too"
        );

        let n = g.challenge(1, &t1, now).unwrap();
        assert_eq!(
            g.take_challenge(2, &t1, &n, now),
            Err("no challenge was issued"),
            "another session"
        );
        let wrong = Nonce::new("B".repeat(43)).unwrap();
        assert_eq!(
            g.take_challenge(1, &t1, &wrong, now),
            Err("challenge does not match")
        );

        let old = g.challenge(1, &t1, now).unwrap();
        let new = g.challenge(1, &t1, now).unwrap();
        assert_ne!(old, new);
        assert!(g.take_challenge(1, &t1, &old, now).is_err(), "replaced");

        let n = g.challenge(1, &t1, now).unwrap();
        assert_eq!(
            g.take_challenge(1, &t1, &n, now + CHALLENGE_TTL),
            Err("challenge expired")
        );
    }

    #[test]
    fn a_grant_covers_one_terminal_in_one_session_for_its_ttl() {
        let mut g = Grants::default();
        let now = Instant::now();
        let (t1, t2) = (tid("term_1"), tid("term_2"));
        let k = key('K');
        assert!(!g.granted(1, &t1, &k, now));
        g.grant(1, &t1, &k, now, GRANT_TTL);
        assert!(g.granted(1, &t1, &k, now));
        assert!(g.granted(1, &t1, &k, now + GRANT_TTL - Duration::from_secs(1)));
        assert!(
            !g.granted(1, &t1, &k, now + GRANT_TTL),
            "absolute, not extended"
        );
        g.grant(1, &t1, &k, now, GRANT_TTL);
        assert!(!g.granted(1, &t2, &k, now), "another terminal");
        assert!(!g.granted(2, &t1, &k, now), "another session");
        assert!(!g.granted(1, &t1, &key('L'), now), "another key");
        assert!(!g.is_empty());

        g.grant(1, &t2, &k, now, GRANT_TTL);
        g.drop_terminal("term_1");
        assert!(!g.granted(1, &t1, &k, now));
        assert!(g.granted(1, &t2, &k, now));
        g.end_session(1);
        assert!(!g.granted(1, &t2, &k, now));
        assert!(g.is_empty());
    }

    #[test]
    fn grants_per_session_are_capped() {
        let mut g = Grants::default();
        let now = Instant::now();
        let k = key('K');
        for i in 0..=MAX_GRANTS {
            g.grant(1, &tid(&format!("term_{i}")), &k, now, GRANT_TTL);
        }
        assert!(!g.granted(1, &tid("term_0"), &k, now), "the oldest goes");
        assert!(g.granted(1, &tid(&format!("term_{MAX_GRANTS}")), &k, now));
        assert_eq!(g.sessions[&1].grants.len(), MAX_GRANTS);
    }

    #[test]
    fn verifies_the_phone_signature() {
        let pkcs8 = collie_tls::generate().unwrap();
        let signing = collie_tls::load(&pkcs8).unwrap();
        let spki = signing.public_key().unwrap().as_ref().to_vec();
        let phone = TerminalKey::new(URL_SAFE_NO_PAD.encode(&spki)).unwrap();
        let t1 = tid("term_1");
        let nonce = Nonce::new("N".repeat(43)).unwrap();
        let sign = |node: &str, t: &TerminalId| {
            let message = protocol::terminal_grant_message(node, t, &nonce);
            let der = signing
                .choose_scheme(&[collie_tls::SCHEME])
                .unwrap()
                .sign(&message)
                .unwrap();
            Signature::new(URL_SAFE_NO_PAD.encode(der)).unwrap()
        };
        let good = sign("nMAC", &t1);
        assert!(verify(&phone, "nMAC", &t1, &nonce, &good));
        assert!(
            !verify(&phone, "nOTHER", &t1, &nonce, &good),
            "another machine"
        );
        assert!(!verify(&phone, "nMAC", &tid("term_2"), &nonce, &good));
        let other = Nonce::new("O".repeat(43)).unwrap();
        assert!(!verify(&phone, "nMAC", &t1, &other, &good));
        assert!(!verify(
            &phone,
            "nMAC",
            &t1,
            &nonce,
            &sign("nMAC", &tid("term_2"))
        ));
        assert!(
            !verify(&key('K'), "nMAC", &t1, &nonce, &good),
            "not a P-256 key"
        );
    }
}
