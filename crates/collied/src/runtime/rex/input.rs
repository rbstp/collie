//! Pastes and named keys to a listed agent's block, as `block.event`s on the follower's
//! control connection: the server takes them only from a connection attached to the block's
//! session, and runs them in the order sent.

use serde::Serialize;
use serde_json::json;

use super::Rex;
use super::client::{Method, TIMEOUT};
use crate::runtime::Error;

/// The only events collied sends: a paste, which Rex brackets when the program turned
/// bracketed paste on and in which an ESC arrives as a space, and a key press.
#[derive(Serialize)]
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
enum Event<'a> {
    Paste { text: &'a str },
    Key(Press),
}

/// A key as Rex's `key` event takes it: a W3C key code, Rex's modifier string and the text
/// the key types on a US keyboard.
#[derive(Serialize)]
struct Press {
    key: String,
    mods: String,
    consumed_mods: &'static str,
    text: String,
    unshifted_codepoint: u32,
    composing: bool,
    action: &'static str,
}

pub const STATUS_CHANGED: &str = "status_changed";

pub(super) const REFUSED: &str =
    "Rex agents take no cmd key, and no ctrl key that Rex would send as nothing or as ESC [";

/// Control without Alt on these sends nothing in Rex (build 1972); herdr-core refuses them.
const CONTROL_SILENT: &[u8] = b"i',-.;=`";

/// A key name in the syntax herdr takes (`ctrl+alt+x`, `shift+f5`), as the bytes herdr-core
/// sends a program with no keyboard protocol, checked live for every name collie takes. What
/// Rex cannot send that way is refused rather than sent as nothing: `cmd`, and Control on `[`,
/// on a character with Shift, or on those in [`CONTROL_SILENT`]. Control and Alt on `[`
/// arrive as `ESC [`, which the next keys could make an escape sequence.
fn press(name: &str) -> Result<Press, Error> {
    let unknown = || Error::Unsupported(format!("no such key: {}", crate::printable(name)));
    let mut parts: Vec<&str> = name.split('+').collect();
    let key = parts.pop().ok_or_else(unknown)?;
    let has = |m: &str| parts.contains(&m);
    if parts
        .iter()
        .any(|m| !matches!(*m, "ctrl" | "alt" | "shift" | "cmd"))
    {
        return Err(unknown());
    }
    if has("cmd") {
        return Err(Error::Unsupported(REFUSED.to_owned()));
    }
    let (code, text, codepoint): (String, String, u32) = match key {
        "enter" => ("enter".into(), "\r".into(), 13),
        "esc" => ("escape".into(), "\u{1b}".into(), 27),
        "tab" => ("tab".into(), "\t".into(), 9),
        "backspace" => ("backspace".into(), "\u{7f}".into(), 127),
        "space" if has("ctrl") && has("shift") && !has("alt") => {
            return Err(Error::Unsupported(REFUSED.to_owned()));
        }
        "space" => ("space".into(), " ".into(), 32),
        "up" | "down" | "left" | "right" => (format!("arrow_{key}"), String::new(), 0),
        "end" => ("end".into(), String::new(), 0),
        _ if function(key) => (key.into(), String::new(), 0),
        _ => {
            let [c] = *key.as_bytes() else {
                return Err(unknown());
            };
            let silent =
                has("ctrl") && !has("alt") && (has("shift") || CONTROL_SILENT.contains(&c));
            if (has("ctrl") && c == b'[') || silent {
                return Err(Error::Unsupported(REFUSED.to_owned()));
            }
            let code = match c {
                b'a'..=b'z' => format!("key_{key}"),
                b'0'..=b'9' => format!("digit_{key}"),
                _ => punctuation(c).ok_or_else(unknown)?.to_owned(),
            };
            // As herdr-core: Shift makes a letter a capital and leaves anything else as is.
            let text = if has("shift") {
                c.to_ascii_uppercase()
            } else {
                c
            };
            (code, char::from(text).to_string(), u32::from(c))
        }
    };
    // Control and Alt make the bytes from the code and the modifiers, not the text.
    let text = if has("ctrl") || has("alt") {
        String::new()
    } else {
        text
    };
    let mods: Vec<&str> = ["shift", "ctrl", "alt"]
        .into_iter()
        .filter(|m| has(m))
        .collect();
    Ok(Press {
        key: code,
        mods: mods.join("+"),
        consumed_mods: "",
        text,
        unshifted_codepoint: codepoint,
        composing: false,
        action: "press",
    })
}

fn function(key: &str) -> bool {
    key.strip_prefix('f').is_some_and(|n| {
        !n.starts_with('0') && n.parse::<u8>().is_ok_and(|n| (1..=12).contains(&n))
    })
}

/// A US keyboard's code for the key.
fn punctuation(c: u8) -> Option<&'static str> {
    Some(match c {
        b'`' => "backquote",
        b'\\' => "backslash",
        b'[' => "bracket_left",
        b']' => "bracket_right",
        b',' => "comma",
        b'-' => "minus",
        b'.' => "period",
        b'\'' => "quote",
        b';' => "semicolon",
        b'/' => "slash",
        b'=' => "equal",
        _ => return None,
    })
}

/// The status the ready check saw, which each event of a call requires again: Rex has no
/// conditional send. `seq` also pins a blocked agent's dialog, so an answer never lands in
/// the next one.
pub struct Seen<'a> {
    pub status: &'a str,
    pub seq: Option<u64>,
}

impl Rex {
    /// From the model, with no call: the pump applies every report Rex pushed before an
    /// event's response before that response returns.
    fn still(&self, id: &str, seen: &Seen) -> Result<(), Error> {
        let a = self.get(id)?;
        if a.agent_status != seen.status || seen.seq.is_some_and(|s| s != a.state_change_seq) {
            return Err(Error::Rex {
                code: STATUS_CHANGED.to_owned(),
                message: "the agent's status changed".to_owned(),
            });
        }
        Ok(())
    }

    async fn send(
        &self,
        id: &str,
        events: &[Event<'_>],
        seen: Option<&Seen<'_>>,
    ) -> Result<(), Error> {
        let _one = self.input.lock().await;
        let (client, session, block, _) = self.target(id)?;
        for event in events {
            if let Some(seen) = seen {
                self.still(id, seen)?;
            }
            let payload = json!({ "session_id": session, "block_id": block, "event": event });
            client
                .call::<serde_json::Value>(Method::BlockEvent, payload, TIMEOUT)
                .await?;
        }
        Ok(())
    }

    /// Pasted, then Enter when `enter`.
    pub async fn paste(
        &self,
        id: &str,
        text: &str,
        enter: bool,
        seen: Option<&Seen<'_>>,
    ) -> Result<(), Error> {
        let mut events = vec![Event::Paste { text }];
        if enter {
            events.push(Event::Key(press("enter")?));
        }
        self.send(id, &events, seen).await
    }

    /// Every key is checked before the first is sent.
    pub async fn keys(&self, id: &str, keys: &[&str]) -> Result<(), Error> {
        let events = (keys.iter())
            .map(|k| press(k).map(Event::Key))
            .collect::<Result<Vec<_>, _>>()?;
        self.send(id, &events, None).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_map_to_rex_codes_modifiers_and_text() {
        let p = |name| {
            let p = press(name).unwrap();
            (p.key, p.mods, p.text, p.unshifted_codepoint)
        };
        let s = String::from;
        assert_eq!(p("enter"), (s("enter"), s(""), s("\r"), 13));
        assert_eq!(p("ctrl+enter"), (s("enter"), s("ctrl"), s(""), 13));
        assert_eq!(p("shift+tab"), (s("tab"), s("shift"), s("\t"), 9));
        assert_eq!(p("ctrl+alt+x"), (s("key_x"), s("ctrl+alt"), s(""), 120));
        assert_eq!(p("alt+shift+x"), (s("key_x"), s("shift+alt"), s(""), 120));
        assert_eq!(p("shift+a"), (s("key_a"), s("shift"), s("A"), 97));
        assert_eq!(p("shift+1"), (s("digit_1"), s("shift"), s("1"), 49));
        assert_eq!(p("shift+/"), (s("slash"), s("shift"), s("/"), 47));
        assert_eq!(p("`"), (s("backquote"), s(""), s("`"), 96));
        assert_eq!(p("shift+f5"), (s("f5"), s("shift"), s(""), 0));
        assert_eq!(p("ctrl+end"), (s("end"), s("ctrl"), s(""), 0));
        assert_eq!(p("left"), (s("arrow_left"), s(""), s(""), 0));
    }

    #[test]
    fn keys_rex_cannot_send_as_herdr_does_are_refused() {
        for name in [
            "cmd+k",
            "alt+cmd+x",
            "ctrl+[",
            "ctrl+alt+[",
            "ctrl+alt+shift+[",
            "ctrl+shift+a",
            "ctrl+shift+1",
            "ctrl+i",
            "ctrl+`",
        ] {
            assert!(
                matches!(press(name), Err(Error::Unsupported(w)) if w == REFUSED),
                "{name}"
            );
        }
        // Each key Rex sends as nothing with Control, one by one, and Shift's space.
        for c in CONTROL_SILENT.iter().map(|c| char::from(*c)) {
            let name = format!("ctrl+{c}");
            assert!(
                matches!(press(&name), Err(Error::Unsupported(w)) if w == REFUSED),
                "{name}"
            );
        }
        assert_eq!(CONTROL_SILENT, b"i',-.;=`");
        assert!(matches!(press("ctrl+shift+space"), Err(Error::Unsupported(w)) if w == REFUSED));
        assert!(press("ctrl+alt+shift+space").is_ok() && press("ctrl+space").is_ok());
        for name in ["", "+", "super+x", "home", "f13", "ctrl+", "ab", "~"] {
            assert!(
                matches!(press(name), Err(Error::Unsupported(w)) if w.starts_with("no such key")),
                "{name:?}"
            );
        }
        for name in [
            "alt+[",
            "ctrl+]",
            "ctrl+alt+i",
            "ctrl+shift+up",
            "ctrl+alt+shift+x",
        ] {
            assert!(press(name).is_ok(), "{name}");
        }
    }
}
