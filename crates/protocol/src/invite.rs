use crate::ids::{InvalidValue, KeyPin, PairingCode};

pub const DEFAULT_PORT: u16 = 8457;
pub const WS_PATH: &str = "/collie/v1";
pub const WS_SUBPROTOCOL: &str = "collie.v1";

/// Content of the pairing QR. The code rides in the URI fragment so it never
/// lands in a request line or a log if the URI is ever opened by something else.
#[derive(Clone, PartialEq, Eq)]
pub struct PairingInvite {
    pub host: String,
    pub port: u16,
    pub node_id: String,
    pub key: KeyPin,
    pub code: PairingCode,
}

impl std::fmt::Debug for PairingInvite {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PairingInvite")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("node_id", &self.node_id)
            .field("key", &self.key)
            .finish_non_exhaustive()
    }
}

const SCHEME: &str = "collie://pair#";

impl PairingInvite {
    pub fn to_uri(&self) -> String {
        format!(
            "{SCHEME}v=2&h={}&p={}&n={}&k={}&c={}",
            self.host,
            self.port,
            self.node_id,
            self.key.as_str(),
            self.code.as_str()
        )
    }

    pub fn parse(uri: &str) -> Result<Self, InvalidValue> {
        let err = InvalidValue("PairingInvite");
        let rest = uri.trim().strip_prefix(SCHEME).ok_or(err)?;
        let (mut v, mut host, mut port, mut node, mut key, mut code) =
            (None, None, None, None, None, None);
        for pair in rest.split('&') {
            let (k, val) = pair.split_once('=').ok_or(err)?;
            let slot = match k {
                "v" => &mut v,
                "h" => &mut host,
                "p" => &mut port,
                "n" => &mut node,
                "k" => &mut key,
                "c" => &mut code,
                _ => return Err(err),
            };
            if slot.replace(val).is_some() {
                return Err(err);
            }
        }
        if v != Some("2") {
            return Err(err);
        }
        let host = host.filter(|h| is_hostname(h)).ok_or(err)?;
        let node_id = node.filter(|n| is_node_id(n)).ok_or(err)?;
        let port = port
            .and_then(|p| p.parse::<u16>().ok())
            .filter(|p| *p != 0)
            .ok_or(err)?;
        let key = KeyPin::new(key.ok_or(err)?)?;
        let code = PairingCode::new(code.ok_or(err)?)?;
        Ok(Self {
            host: host.to_owned(),
            port,
            node_id: node_id.to_owned(),
            key,
            code,
        })
    }
}

fn is_hostname(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 253
        && s.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        })
}

fn is_node_id(s: &str) -> bool {
    !s.is_empty() && s.len() <= 64 && s.bytes().all(|b| b.is_ascii_alphanumeric())
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    fn invite() -> PairingInvite {
        PairingInvite {
            host: "collie-work-mac.tail1234.ts.net".into(),
            port: DEFAULT_PORT,
            node_id: "nAbC123CNTRL".into(),
            key: KeyPin::new("K".repeat(43)).unwrap(),
            code: PairingCode::new("Zm9vYmFyYmF6cXV4cXV1dQ").unwrap(),
        }
    }

    #[test]
    fn round_trip() {
        let uri = invite().to_uri();
        assert!(uri.starts_with("collie://pair#v=2&"));
        assert_eq!(PairingInvite::parse(&uri).unwrap(), invite());
    }

    #[test]
    fn rejects_tampering() {
        let uri = invite().to_uri();
        for bad in [
            uri.replace("v=2", "v=1"),
            uri.replace(&format!("&k={}", "K".repeat(43)), ""),
            uri.replace(&"K".repeat(43), &"K".repeat(42)),
            uri.replace("collie://pair#", "https://pair#"),
            uri.replace(".ts.net", ".ts.net/evil"),
            uri.replace("p=8457", "p=0"),
            uri.replace("n=nAbC123CNTRL", "n=n%2F"),
            format!("{uri}&c=AAAAAAAAAAAAAAAAAAAAAA"),
            format!("{uri}&x=1"),
            uri.replace("&c=", "&k="),
        ] {
            assert!(PairingInvite::parse(&bad).is_err(), "{bad}");
        }
    }

    proptest! {
        #[test]
        fn parse_never_panics(
            uri in "\\PC*",
            fields in "(collie://pair#)?([vhpnkcx]=[A-Za-z0-9_.%-]{0,50}&?){0,8}",
        ) {
            let _ = PairingInvite::parse(&uri);
            let _ = PairingInvite::parse(&fields);
        }

        #[test]
        fn parse_reads_back_to_uri(
            host in "[a-zA-Z0-9]([a-zA-Z0-9-]{0,61}[a-zA-Z0-9])?(\\.[a-zA-Z0-9]([a-zA-Z0-9-]{0,61}[a-zA-Z0-9])?){0,2}",
            port in 1..=u16::MAX,
            node_id in "[a-zA-Z0-9]{1,64}",
            key in "[A-Za-z0-9_-]{43}",
            code in "[A-Za-z0-9_-]{22}",
        ) {
            let invite = PairingInvite {
                host,
                port,
                node_id,
                key: KeyPin::new(key).unwrap(),
                code: PairingCode::new(code).unwrap(),
            };
            prop_assert_eq!(PairingInvite::parse(&invite.to_uri()), Ok(invite));
        }
    }

    #[test]
    fn debug_hides_code() {
        assert!(!format!("{:?}", invite()).contains("Zm9v"));
    }
}
