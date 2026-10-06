use std::collections::HashMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::ids::TerminalId;
use crate::messages::{ReadSource, TerminalRead};

/// The watched agent's new `recent` text: `head`, then lines `skip..skip + keep` of the
/// text the watch sent last, then `tail`, joined with `\n`. `base` is [`text_hash`] of
/// that previous text. A client that does not hold it re-issues `agent.watch`, whose
/// first event is always a full `agent.output`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct OutputPatch {
    pub terminal_id: TerminalId,
    pub base: u64,
    pub head: Vec<String>,
    pub skip: u32,
    pub keep: u32,
    pub tail: Vec<String>,
    pub truncated: bool,
    /// [`TerminalRead::wraps`] and [`TerminalRead::splits`] of the new text, absent when
    /// they are those of the previous text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wraps: Option<Vec<u32>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub splits: Option<Vec<u32>>,
}

impl OutputPatch {
    /// `None` unless the patch carries at most half of `next`'s bytes.
    pub fn between(prev: &TerminalRead, next: &TerminalRead) -> Option<Self> {
        let old: Vec<&str> = prev.ansi.split('\n').collect();
        let new: Vec<&str> = next.ansi.split('\n').collect();
        let (start, skip, keep) = longest_run(&old, &new);
        let head = &new[..start];
        let tail = &new[start + keep..];
        let sent: usize = head.iter().chain(tail).map(|l| l.len() + 1).sum();
        if keep == 0 || sent > next.ansi.len() / 2 {
            return None;
        }
        Some(Self {
            terminal_id: next.terminal_id.clone(),
            base: text_hash(&prev.ansi),
            head: head.iter().map(|&l| l.to_owned()).collect(),
            skip: u32::try_from(skip).ok()?,
            keep: u32::try_from(keep).ok()?,
            tail: tail.iter().map(|&l| l.to_owned()).collect(),
            truncated: next.truncated,
            wraps: (next.wraps != prev.wraps).then(|| next.wraps.clone()),
            splits: (next.splits != prev.splits).then(|| next.splits.clone()),
        })
    }

    /// `None` when `prev` is not the text the patch was made against.
    pub fn apply(&self, prev: &TerminalRead) -> Option<TerminalRead> {
        if text_hash(&prev.ansi) != self.base {
            return None;
        }
        let skip = usize::try_from(self.skip).ok()?;
        let end = skip.checked_add(usize::try_from(self.keep).ok()?)?;
        let old: Vec<&str> = prev.ansi.split('\n').collect();
        let kept = old.get(skip..end)?;
        let lines: Vec<&str> = self
            .head
            .iter()
            .map(String::as_str)
            .chain(kept.iter().copied())
            .chain(self.tail.iter().map(String::as_str))
            .collect();
        Some(TerminalRead {
            terminal_id: self.terminal_id.clone(),
            source: ReadSource::Recent,
            ansi: lines.join("\n"),
            truncated: self.truncated,
            wraps: self.wraps.clone().unwrap_or_else(|| prev.wraps.clone()),
            splits: self.splits.clone().unwrap_or_else(|| prev.splits.clone()),
        })
    }
}

/// 64-bit FNV-1a: stable across builds, so collied and the app agree on it. It only
/// detects a patch applied to the wrong text; the channel itself is authenticated.
pub fn text_hash(text: &str) -> u64 {
    text.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
    })
}

/// The longest run of lines `new` shares with `old`, as `(start in new, start in old, length)`.
fn longest_run(old: &[&str], new: &[&str]) -> (usize, usize, usize) {
    let mut at: HashMap<&str, Vec<usize>> = HashMap::new();
    for (i, &line) in old.iter().enumerate() {
        at.entry(line).or_default().push(i);
    }
    let mut best = (0, 0, 0);
    for (s, &line) in new.iter().enumerate() {
        if new.len() - s <= best.2 {
            break;
        }
        for &d in at.get(line).map_or(&[][..], Vec::as_slice) {
            // A run that also matches one line earlier was already measured from there.
            if s > 0 && d > 0 && old[d - 1] == new[s - 1] {
                continue;
            }
            let len = old[d..]
                .iter()
                .zip(&new[s..])
                .take_while(|(a, b)| a == b)
                .count();
            if len > best.2 {
                best = (s, d, len);
            }
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(ansi: &str) -> TerminalRead {
        TerminalRead {
            terminal_id: TerminalId::new("t1").unwrap(),
            source: ReadSource::Recent,
            ansi: ansi.to_owned(),
            truncated: false,
            wraps: Vec::new(),
            splits: Vec::new(),
        }
    }

    fn history(range: std::ops::Range<u32>) -> String {
        range
            .map(|i| format!("\u{1b}[1mline {i}\u{1b}[0m\r\n"))
            .collect()
    }

    fn round_trip(prev: &str, next: &str) -> OutputPatch {
        let patch = OutputPatch::between(&read(prev), &read(next)).expect("a patch");
        assert_eq!(patch.apply(&read(prev)), Some(read(next)));
        let json = serde_json::to_string(&patch).unwrap();
        assert_eq!(serde_json::from_str::<OutputPatch>(&json).unwrap(), patch);
        patch
    }

    #[test]
    fn a_changed_status_line_sends_only_the_tail() {
        let prev = history(0..1000) + "✻ Thinking… (3s)\r\n";
        let next = history(0..1000) + "✶ Thinking… (4s)\r\n";
        let patch = round_trip(&prev, &next);
        assert_eq!((patch.head.len(), patch.skip, patch.keep), (0, 0, 1000));
        assert_eq!(patch.tail, ["✶ Thinking… (4s)\r", ""]);
    }

    #[test]
    fn scrolled_history_is_kept_by_reference() {
        let prev = history(0..1000) + "> \r\n";
        let next = history(3..1003) + "> \r\n";
        let patch = round_trip(&prev, &next);
        assert_eq!((patch.head.len(), patch.skip, patch.keep), (0, 3, 997));
        let json = serde_json::to_string(&patch).unwrap();
        assert!(json.len() < 300, "{} bytes", json.len());
    }

    #[test]
    fn a_cut_first_line_goes_in_the_head() {
        let prev = history(0..1000);
        let next = format!("ine 4\u{1b}[0m\r\n{}", history(5..1005));
        let patch = round_trip(&prev, &next);
        assert_eq!(patch.head, ["ine 4\u{1b}[0m\r"]);
        assert_eq!((patch.skip, patch.keep), (5, 995));
    }

    #[test]
    fn wraps_ride_along_when_they_change() {
        let prev = TerminalRead {
            wraps: vec![7],
            splits: vec![9],
            ..read(&(history(0..100) + "a"))
        };
        let next = TerminalRead {
            wraps: vec![7, 100],
            ..read(&(history(0..100) + "a\r\nb\r\nc"))
        };
        let patch = OutputPatch::between(&prev, &next).unwrap();
        assert_eq!(
            (patch.wraps.as_deref(), patch.splits.as_deref()),
            (Some(&[7, 100][..]), Some(&[][..]))
        );
        assert_eq!(patch.apply(&prev), Some(next.clone()));

        let tick = TerminalRead {
            ansi: next.ansi.clone() + "d",
            ..next.clone()
        };
        let patch = OutputPatch::between(&next, &tick).unwrap();
        assert_eq!((&patch.wraps, &patch.splits), (&None, &None));
        assert!(!serde_json::to_string(&patch).unwrap().contains("wraps"));
        assert_eq!(patch.apply(&next), Some(tick));
    }

    #[test]
    fn mostly_new_text_is_sent_whole() {
        let prev = read(&history(0..10));
        assert!(OutputPatch::between(&prev, &read(&history(100..110))).is_none());
        assert!(OutputPatch::between(&prev, &read(&history(6..16))).is_none());
    }

    #[test]
    fn repeated_lines_round_trip() {
        let blank = "\r\n".repeat(400);
        let prev = format!("{blank}a\r\n{blank}b\r\n{blank}");
        let next = format!("{blank}b\r\n{blank}c\r\n{blank}");
        round_trip(&prev, &next);
        round_trip(&"x\n".repeat(1000), &("x\n".repeat(999) + "y"));
    }

    #[test]
    fn apply_needs_the_base_text() {
        let prev = history(0..100) + "a";
        let patch = round_trip(&prev, &(history(0..100) + "b"));
        assert_eq!(patch.apply(&read(&(history(0..100) + "c"))), None);
        let past_end = OutputPatch {
            keep: 1000,
            ..patch.clone()
        };
        assert_eq!(past_end.apply(&read(&prev)), None);
        let overflow = OutputPatch {
            skip: u32::MAX,
            keep: u32::MAX,
            ..patch
        };
        assert_eq!(overflow.apply(&read(&prev)), None);
    }

    #[test]
    fn hash_is_fnv1a() {
        assert_eq!(text_hash(""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(text_hash("a"), 0xaf63_dc4c_8601_ec8c);
    }
}
