use std::collections::HashMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::ids::TerminalId;
use crate::messages::{ReadSource, TerminalRead};

/// The watched agent's new `recent` text: `head`, then lines `skip..skip + keep` of the
/// text the watch sent last, then `tail`, then the last `trail` lines of that text, joined
/// with `\n`. The trailing lines start at or after `skip + keep`. `base` is [`text_hash`] of
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
    pub trail: u32,
    pub truncated: bool,
    /// [`TerminalRead::wraps`] and [`TerminalRead::splits`] of the new text, absent when
    /// they are those of the previous text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wraps: Option<Vec<u32>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub splits: Option<Vec<u32>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub copilot_scrollbar: Option<bool>,
}

impl OutputPatch {
    /// `None` unless the patch carries at most half of `next`'s bytes.
    pub fn between(prev: &TerminalRead, next: &TerminalRead) -> Option<Self> {
        let old: Vec<&str> = prev.ansi.split('\n').collect();
        let new: Vec<&str> = next.ansi.split('\n').collect();
        let (start, skip, keep) = longest_run(&old, &new);
        let head = &new[..start];
        let rest = &new[start + keep..];
        let trail = rest
            .iter()
            .rev()
            .zip(old[skip + keep..].iter().rev())
            .take_while(|(a, b)| a == b)
            .count();
        let tail = &rest[..rest.len() - trail];
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
            trail: u32::try_from(trail).ok()?,
            truncated: next.truncated,
            wraps: (next.wraps != prev.wraps).then(|| next.wraps.clone()),
            splits: (next.splits != prev.splits).then(|| next.splits.clone()),
            copilot_scrollbar: (next.copilot_scrollbar != prev.copilot_scrollbar)
                .then_some(next.copilot_scrollbar),
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
        let from = old.len().checked_sub(usize::try_from(self.trail).ok()?)?;
        if from < end {
            return None;
        }
        let lines: Vec<&str> = self
            .head
            .iter()
            .map(String::as_str)
            .chain(kept.iter().copied())
            .chain(self.tail.iter().map(String::as_str))
            .chain(old[from..].iter().copied())
            .collect();
        Some(TerminalRead {
            terminal_id: self.terminal_id.clone(),
            source: ReadSource::Recent,
            ansi: lines.join("\n"),
            truncated: self.truncated,
            wraps: self.wraps.clone().unwrap_or_else(|| prev.wraps.clone()),
            splits: self.splits.clone().unwrap_or_else(|| prev.splits.clone()),
            copilot_scrollbar: self.copilot_scrollbar.unwrap_or(prev.copilot_scrollbar),
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
            copilot_scrollbar: false,
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
        assert_eq!(patch.tail, ["✶ Thinking… (4s)\r"]);
        assert_eq!(patch.trail, 1);
    }

    #[test]
    fn unchanged_lines_below_a_change_are_kept_by_reference() {
        let input_box = "─".repeat(120) + "\r\n> \r\n" + &"─".repeat(120) + "\r\n  ? for shortcuts";
        let prev = history(0..200) + "✻ Thinking… (3s)\r\n\r\n" + &input_box;
        let next = history(0..200) + "✶ Thinking… (4s)\r\n\r\n" + &input_box;
        let patch = round_trip(&prev, &next);
        assert_eq!((patch.skip, patch.keep, patch.trail), (0, 200, 5));
        assert_eq!(patch.tail, ["✶ Thinking… (4s)\r"]);
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
            copilot_scrollbar: true,
            ..read(&(history(0..100) + "a\r\nb\r\nc"))
        };
        let patch = OutputPatch::between(&prev, &next).unwrap();
        assert_eq!(
            (patch.wraps.as_deref(), patch.splits.as_deref()),
            (Some(&[7, 100][..]), Some(&[][..]))
        );
        assert_eq!(patch.apply(&prev), Some(next.clone()));
        assert_eq!(patch.copilot_scrollbar, Some(true));

        let tick = TerminalRead {
            ansi: next.ansi.clone() + "d",
            ..next.clone()
        };
        let patch = OutputPatch::between(&next, &tick).unwrap();
        assert_eq!(
            (&patch.wraps, &patch.splits, patch.copilot_scrollbar),
            (&None, &None, None)
        );
        assert!(!serde_json::to_string(&patch).unwrap().contains("wraps"));
        assert_eq!(patch.apply(&next), Some(tick));
    }

    #[test]
    fn mostly_new_text_is_sent_whole() {
        let prev = read(&history(0..10));
        assert!(OutputPatch::between(&prev, &read(&history(100..110))).is_none());
        assert!(OutputPatch::between(&prev, &read(&history(6..16))).is_none());
        let changed = history(0..3) + &history(100..106) + &history(8..10);
        assert!(OutputPatch::between(&prev, &read(&changed)).is_none());
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

    /// xorshift64: a fixed seed, so a failure reproduces.
    struct Rng(u64);

    impl Rng {
        fn below(&mut self, n: usize) -> usize {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            usize::try_from(self.0 % n as u64).unwrap()
        }

        /// Lines from a small alphabet, so texts share runs, repeats and trailing lines.
        fn text(&mut self) -> Vec<String> {
            (0..self.below(40))
                .map(|_| ["", "─────", "> ", "a", "b", "✻ c"][self.below(6)].to_owned())
                .collect()
        }

        fn edit(&mut self, lines: &[String]) -> Vec<String> {
            let mut out = lines.to_vec();
            for _ in 0..self.below(4) {
                let at = self.below(out.len() + 1);
                match self.below(3) {
                    0 => out.insert(at, format!("new {}", self.below(9))),
                    1 if at < out.len() => drop(out.remove(at)),
                    _ => out.drain(..at.min(self.below(3))).for_each(drop),
                }
            }
            out
        }
    }

    #[test]
    fn apply_of_between_is_the_new_text() {
        let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
        let (mut patched, mut trailed) = (0, 0);
        for _ in 0..20_000 {
            let a = rng.text();
            let b = if rng.below(4) == 0 {
                rng.text()
            } else {
                rng.edit(&a)
            };
            let (prev, next) = (read(&a.join("\n")), read(&b.join("\n")));
            let Some(patch) = OutputPatch::between(&prev, &next) else {
                continue;
            };
            assert_eq!(patch.apply(&prev).as_ref(), Some(&next), "{a:?} -> {b:?}");
            let sent: usize = patch
                .head
                .iter()
                .chain(&patch.tail)
                .map(|l| l.len() + 1)
                .sum();
            assert!(sent <= next.ansi.len() / 2, "{a:?} -> {b:?}");
            patched += 1;
            trailed += usize::from(patch.trail > 0);
        }
        assert!(patched > 1000 && trailed > 1000, "{patched} {trailed}");
    }

    #[test]
    fn out_of_range_runs_are_refused() {
        let mut rng = Rng(0x2545_f491_4f6c_dd1d);
        let picks = [0, 1, 2, 3, 5, 8, 13, u32::MAX - 1, u32::MAX];
        let mut applied = 0;
        for _ in 0..20_000 {
            let lines = rng.text();
            let old = read(&lines.join("\n"));
            let n = old.ansi.split('\n').count() as u64;
            let [skip, keep, trail] = [0; 3].map(|_| picks[rng.below(picks.len())]);
            let patch = OutputPatch {
                terminal_id: old.terminal_id.clone(),
                base: text_hash(&old.ansi),
                head: vec!["h".into()],
                skip,
                keep,
                tail: vec!["t".into()],
                trail,
                truncated: false,
                wraps: None,
                splits: None,
                copilot_scrollbar: None,
            };
            let end = u64::from(skip) + u64::from(keep);
            let valid = end <= n && u64::from(trail) <= n - end;
            assert_eq!(
                patch.apply(&old).is_some(),
                valid,
                "{skip} {keep} {trail} of {n}"
            );
            applied += usize::from(valid);
        }
        assert!(applied > 1000);
    }

    #[test]
    fn hash_is_fnv1a() {
        assert_eq!(text_hash(""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(text_hash("a"), 0xaf63_dc4c_8601_ec8c);
    }
}
