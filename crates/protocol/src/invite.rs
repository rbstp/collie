use crate::ids::{InvalidValue, PairingCode};

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
    pub code: PairingCode,
}

impl std::fmt::Debug for PairingInvite {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PairingInvite")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("node_id", &self.node_id)
            .finish_non_exhaustive()
    }
}

const SCHEME: &str = "collie://pair#";

impl PairingInvite {
    pub fn to_uri(&self) -> String {
        format!(
            "{SCHEME}v=1&h={}&p={}&n={}&c={}",
            self.host,
            self.port,
            self.node_id,
            self.code.as_str()
        )
    }

    pub fn parse(uri: &str) -> Result<Self, InvalidValue> {
        let err = InvalidValue("PairingInvite");
        let rest = uri.trim().strip_prefix(SCHEME).ok_or(err)?;
        let (mut v, mut host, mut port, mut node, mut code) = (None, None, None, None, None);
        for pair in rest.split('&') {
            let (k, val) = pair.split_once('=').ok_or(err)?;
            let slot = match k {
                "v" => &mut v,
                "h" => &mut host,
                "p" => &mut port,
                "n" => &mut node,
                "c" => &mut code,
                _ => return Err(err),
            };
            if slot.replace(val).is_some() {
                return Err(err);
            }
        }
        if v != Some("1") {
            return Err(err);
        }
        let host = host.filter(|h| is_hostname(h)).ok_or(err)?;
        let node_id = node.filter(|n| is_node_id(n)).ok_or(err)?;
        let port = port
            .and_then(|p| p.parse::<u16>().ok())
            .filter(|p| *p != 0)
            .ok_or(err)?;
        let code = PairingCode::new(code.ok_or(err)?)?;
        Ok(Self {
            host: host.to_owned(),
            port,
            node_id: node_id.to_owned(),
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
    use super::*;

    fn invite() -> PairingInvite {
        PairingInvite {
            host: "collie-devolutions496.tail1234.ts.net".into(),
            port: DEFAULT_PORT,
            node_id: "nAbC123CNTRL".into(),
            code: PairingCode::new("Zm9vYmFyYmF6cXV4cXV1dQ").unwrap(),
        }
    }

    #[test]
    fn round_trip() {
        let uri = invite().to_uri();
        assert!(uri.starts_with("collie://pair#v=1&"));
        assert_eq!(PairingInvite::parse(&uri).unwrap(), invite());
    }

    #[test]
    fn rejects_tampering() {
        let uri = invite().to_uri();
        for bad in [
            uri.replace("v=1", "v=2"),
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

    #[test]
    fn debug_hides_code() {
        assert!(!format!("{:?}", invite()).contains("Zm9v"));
    }
}
