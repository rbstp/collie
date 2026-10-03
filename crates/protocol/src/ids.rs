use std::borrow::Cow;
use std::fmt;

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Serialize};

use crate::limits;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidValue(pub &'static str);

impl fmt::Display for InvalidValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid {}", self.0)
    }
}

impl std::error::Error for InvalidValue {}

macro_rules! validated_string {
    ($(#[$meta:meta])* $name:ident, debug = $debug:ident, check = $check:expr, schema = { $($schema:tt)* }) => {
        $(#[$meta])*
        #[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        #[serde(try_from = "String", into = "String")]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Result<Self, InvalidValue> {
                let value = value.into();
                let check: fn(&str) -> bool = $check;
                if check(&value) {
                    Ok(Self(value))
                } else {
                    Err(InvalidValue(stringify!($name)))
                }
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl TryFrom<String> for $name {
            type Error = InvalidValue;
            fn try_from(value: String) -> Result<Self, Self::Error> {
                Self::new(value)
            }
        }

        impl From<$name> for String {
            fn from(value: $name) -> String {
                value.0
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                validated_string!(@debug $debug, self, f, $name)
            }
        }

        impl JsonSchema for $name {
            fn schema_name() -> Cow<'static, str> {
                stringify!($name).into()
            }

            fn json_schema(_: &mut SchemaGenerator) -> Schema {
                json_schema!({ "type": "string", $($schema)* })
            }
        }
    };
    (@debug plain, $self:ident, $f:ident, $name:ident) => {
        write!($f, "{}({:?})", stringify!($name), $self.0)
    };
    (@debug redacted, $self:ident, $f:ident, $name:ident) => {
        write!($f, "{}(<redacted>)", stringify!($name))
    };
}

fn ascii_ident(s: &str, max: usize, extra: &[u8]) -> bool {
    !s.is_empty()
        && s.len() <= max
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || extra.contains(&b))
}

fn base64url_len(s: &str, len: usize) -> bool {
    s.len() == len
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

fn is_unsafe_char(c: char) -> bool {
    c.is_control() && c != '\n' && c != '\t'
}

/// Invisible and bidi formatting characters let a label render as something else,
/// for example a different phone name on the Mac's pairing prompt.
fn is_format(c: char) -> bool {
    matches!(
        c,
        '\u{00AD}'
            | '\u{061C}'
            | '\u{180E}'
            | '\u{200B}'..='\u{200F}'
            | '\u{202A}'..='\u{202E}'
            | '\u{2060}'..='\u{206F}'
            | '\u{FEFF}'
            | '\u{FFF9}'..='\u{FFFB}'
    )
}

fn label(s: &str) -> bool {
    let n = s.chars().count();
    (1..=limits::MAX_LABEL_CHARS).contains(&n)
        && !s.chars().any(|c| c.is_control() || is_format(c))
        && s.trim() == s
}

validated_string!(
    /// herdr `terminal_id`. Stable across pane moves, unlike `pane_id`, so it is
    /// the only pane handle the phone ever sees.
    TerminalId,
    debug = plain,
    check = |s| ascii_ident(s, 64, b"_:.-"),
    schema = { "pattern": "^[A-Za-z0-9_:.-]{1,64}$" }
);

validated_string!(
    WorkspaceId,
    debug = plain,
    check = |s| ascii_ident(s, 64, b"_:.-"),
    schema = { "pattern": "^[A-Za-z0-9_:.-]{1,64}$" }
);

validated_string!(
    ApprovalId,
    debug = plain,
    check = |s| ascii_ident(s, 64, b"_-"),
    schema = { "pattern": "^[A-Za-z0-9_-]{1,64}$" }
);

validated_string!(
    /// ActivityKit `Activity.id`.
    ActivityId,
    debug = plain,
    check = |s| ascii_ident(s, 64, b"-"),
    schema = { "pattern": "^[A-Za-z0-9-]{1,64}$" }
);

validated_string!(
    /// Same grammar as herdr's `agent.start` name/kind.
    AgentKind,
    debug = plain,
    check = |s| s.as_bytes().first().is_some_and(u8::is_ascii_lowercase)
        && s.len() <= 32
        && s.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-'),
    schema = { "pattern": "^[a-z][a-z0-9_-]{0,31}$" }
);

validated_string!(
    /// 32 random bytes, base64url without padding.
    Nonce,
    debug = redacted,
    check = |s| base64url_len(s, 43),
    schema = { "pattern": "^[A-Za-z0-9_-]{43}$" }
);

validated_string!(
    /// 32 random bytes, base64url without padding (canonical: the last character
    /// carries 4 bits): the phone's key for one Mac's encrypted alert context.
    NotificationKey,
    debug = redacted,
    check = |s| base64url_len(s, 43) && b"AEIMQUYcgkosw048".contains(&s.as_bytes()[42]),
    schema = { "pattern": "^[A-Za-z0-9_-]{42}[AEIMQUYcgkosw048]$" }
);

validated_string!(
    /// 16 random bytes, base64url without padding. Lets collied replay the stored
    /// outcome instead of re-running a mutation the client retried after a drop.
    OpId,
    debug = plain,
    check = |s| base64url_len(s, 22),
    schema = { "pattern": "^[A-Za-z0-9_-]{22}$" }
);

validated_string!(
    /// 16 random bytes, base64url without padding. One-time, short-lived.
    PairingCode,
    debug = redacted,
    check = |s| base64url_len(s, 22),
    schema = { "pattern": "^[A-Za-z0-9_-]{22}$" }
);

validated_string!(
    PushToken,
    debug = redacted,
    check = |s| (64..=256).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_hexdigit()),
    schema = { "pattern": "^[0-9A-Fa-f]{64,256}$" }
);

validated_string!(
    Label,
    debug = plain,
    check = label,
    schema = { "minLength": 1, "maxLength": 64 }
);

validated_string!(
    /// Absolute path. collied canonicalizes it and enforces its configured roots.
    Cwd,
    debug = plain,
    check = |s| s.starts_with('/') && s.len() <= limits::MAX_CWD_BYTES && !s.chars().any(char::is_control),
    schema = { "pattern": "^/", "maxLength": 1024 }
);

validated_string!(
    /// Text typed into an agent. ESC and other C0/C1 controls are rejected because
    /// herdr writes prompts as bracketed paste: an embedded `ESC[201~` would end the
    /// paste and let the remainder run as raw keystrokes.
    PromptText,
    debug = plain,
    check = |s| !s.trim().is_empty() && s.len() <= limits::MAX_PROMPT_BYTES && !s.chars().any(is_unsafe_char),
    schema = { "minLength": 1, "maxLength": 32768 }
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_rejects_escape_sequences() {
        assert!(PromptText::new("fix the build\nthen run tests").is_ok());
        assert!(PromptText::new("x\u{1b}[201~rm -rf ~\r").is_err());
        assert!(PromptText::new("x\u{9b}201~").is_err());
        assert!(PromptText::new("   ").is_err());
        assert!(PromptText::new("a".repeat(limits::MAX_PROMPT_BYTES + 1)).is_err());
    }

    #[test]
    fn secrets_are_redacted_in_debug() {
        let nonce = Nonce::new("A".repeat(43)).unwrap();
        assert_eq!(format!("{nonce:?}"), "Nonce(<redacted>)");
        let code = PairingCode::new("B".repeat(22)).unwrap();
        assert!(!format!("{code:?}").contains('B'));
        let key = NotificationKey::new(format!("{}A", "C".repeat(42))).unwrap();
        assert_eq!(format!("{key:?}"), "NotificationKey(<redacted>)");
    }

    #[test]
    fn ids_reject_injection() {
        assert!(TerminalId::new("term_65ce7ae4fd5731").is_ok());
        assert!(TerminalId::new("w6:p1").is_ok());
        assert!(TerminalId::new("w6 p1").is_err());
        assert!(TerminalId::new("").is_err());
        assert!(AgentKind::new("claude").is_ok());
        assert!(AgentKind::new("Claude").is_err());
        assert!(Cwd::new("relative/path").is_err());
        assert!(Label::new(" padded").is_err());
        assert!(Label::new("Richard's iPhone").is_ok());
        assert!(Label::new("iPhone\u{202E}enohPi").is_err());
        assert!(Label::new("i\u{200B}Phone").is_err());
    }

    #[test]
    fn deserialization_validates() {
        assert!(serde_json::from_str::<Nonce>("\"short\"").is_err());
        assert!(serde_json::from_str::<PushToken>(&format!("\"{}\"", "ab".repeat(32))).is_ok());
        assert!(NotificationKey::new("A".repeat(43)).is_ok());
        assert!(NotificationKey::new("A".repeat(42)).is_err());
        assert!(NotificationKey::new(format!("{}B", "A".repeat(42))).is_err());
        assert!(NotificationKey::new(format!("{}=", "A".repeat(42))).is_err());
    }
}
