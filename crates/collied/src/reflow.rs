use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::draft::{Wrap, separator};

enum Prev {
    /// A row here may start a paragraph.
    Start,
    /// The row above: the indent its continuation would have. Its text after that indent is
    /// in `tail`.
    Row(usize),
    /// Rows here may not continue the one above, until a blank row.
    Held,
}

/// The rows of a Claude Code read (split on `\n`) that continue a paragraph it wrapped at the
/// pane's `cols`, as `(wraps, splits)` of a `TerminalRead`. Only prose under an indent-0 `⏺`
/// or `※` row counts; a row that only might continue the one above never does. A shaded sent
/// prompt joins only after a row Claude Code ended with a space in the text's style.
pub fn soft_wraps(ansi: &str, cols: usize) -> (Vec<u32>, Vec<u32>) {
    let (mut wraps, mut splits) = (Vec::new(), Vec::new());
    let mut prose = false;
    let mut prompt: Option<bool> = None;
    let mut prev = Prev::Start;
    let mut wrap = Wrap::default();
    let (mut plain, mut tail) = (String::new(), String::new());
    let rows: Vec<&str> = ansi.split('\n').collect();
    for (i, &row) in rows.iter().enumerate() {
        let mut above = std::mem::take(&mut wrap);
        plain_text(row, &mut plain);
        let trimmed = plain.trim_end();
        let s = trimmed.trim_start_matches(' ');
        let ind = trimmed.len() - s.len();
        if s.is_empty() {
            prev = Prev::Start;
            continue;
        }
        if ind == 0 {
            if let Some(text) = s.strip_prefix("❯ ").filter(|_| shaded_start(row)) {
                (prose, prompt) = (false, Some(ends_in_space(row, false)));
                prev = row_above(&mut tail, text, 2);
                continue;
            }
            prompt = None;
            let body = s.strip_prefix(['⏺', '※']);
            prose = body.is_some();
            prev = match body.map(|b| b.trim_start_matches([' ', '\u{a0}'])) {
                Some(b) if !tool_header(b) => row_above(&mut tail, b, 2),
                _ => Prev::Held,
            };
            continue;
        }
        if let Some(spaced) = prompt.filter(|_| shaded_start(row)) {
            let width = cols.saturating_sub(2);
            if spaced
                && ind == 2
                && matches!(prev, Prev::Row(2))
                && tail.width() <= width
                && separator(&tail, s, width) == " "
            {
                wraps.push(u32::try_from(i).unwrap_or(u32::MAX));
            }
            prompt = Some(ends_in_space(row, false));
            prev = row_above(&mut tail, s, ind);
            continue;
        }
        prompt = None;
        if !prose {
            continue;
        }
        if s.starts_with('⎿') {
            prose = false;
            continue;
        }
        if s.starts_with(|c| ('\u{2500}'..='\u{257f}').contains(&c)) {
            prev = Prev::Start;
            continue;
        }
        if tool_header(s) {
            prev = Prev::Held;
            continue;
        }
        if let Some(len) = list_marker(s) {
            let rest = s[len..].trim_start_matches(' ');
            prev = row_above(&mut tail, rest, ind + s[..s.len() - rest.len()].width());
            continue;
        }
        prev = match prev {
            Prev::Row(hang) if hang == ind => {
                let width = cols.saturating_sub(ind);
                if tail.width() <= width {
                    let at = u32::try_from(i).unwrap_or(u32::MAX);
                    let below = || {
                        rows[i + 1..].iter().map(move |r| {
                            let mut text = String::new();
                            plain_text(r, &mut text);
                            let body = text.trim_end().trim_start_matches(' ');
                            if text.trim_end().len() - body.len() == ind
                                && list_marker(body).is_none()
                                && !body.starts_with('⎿')
                                && !body.starts_with(|c| ('\u{2500}'..='\u{257f}').contains(&c))
                                && !tool_header(body)
                            {
                                body.to_owned()
                            } else {
                                String::new()
                            }
                        })
                    };
                    let row = above;
                    match above.join(&tail, s, below(), width) {
                        " " if wide_word_cut(&tail, s, width)
                            && row.continues(&tail, s, below(), width) =>
                        {
                            splits.push(at);
                            above = row.split(&tail);
                        }
                        " " => wraps.push(at),
                        "" => splits.push(at),
                        _ => {}
                    }
                    wrap = above;
                }
                row_above(&mut tail, s, ind)
            }
            Prev::Start => row_above(&mut tail, s, ind),
            Prev::Row(..) | Prev::Held => Prev::Held,
        };
    }
    (wraps, splits)
}

pub fn soft_wraps_for(kind: &str, ansi: &str, cols: usize) -> (Vec<u32>, Vec<u32>) {
    match kind {
        "claude" => soft_wraps(ansi, cols),
        "codex" | "copilot" => tui_wraps(kind, ansi, cols),
        _ => (Vec::new(), Vec::new()),
    }
}

/// Claude Code draws its logo on true black, which matches a dark terminal on the Mac but not
/// the phone's. Only the standalone escape is replaced; herdr writes each attribute apart.
pub fn without_claude_black(ansi: &str) -> String {
    ansi.replace("\u{1b}[48;2;0;0;0m", "\u{1b}[49m")
}

pub fn without_copilot_scrollbar(ansi: &str, cols: usize) -> String {
    let mut plain = String::new();
    ansi.split('\n')
        .map(|row| {
            plain_text(row, &mut plain);
            let text = plain.trim_end();
            let Some(body) = text.strip_suffix('┃') else {
                return row.to_owned();
            };
            let at = row.rfind('┃').unwrap();
            if text.width() != cols || !body.ends_with(' ') {
                return row.to_owned();
            }
            let content = body
                .strip_prefix("   └ ")
                .filter(|_| row.starts_with("   \u{1b}[0m\u{1b}[38;"))
                .unwrap_or(body);
            if content
                .chars()
                .any(|c| ('\u{2500}'..='\u{257f}').contains(&c))
                && !isolated_scrollbar(row, at)
            {
                return row.to_owned();
            }
            format!("{}{}", &row[..at], &row[at + '┃'.len_utf8()..])
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn copilot_scrollbar_columns(ansi: &str) -> Option<usize> {
    let mut counts = std::collections::BTreeMap::<usize, usize>::new();
    let mut plain = String::new();
    for row in ansi.split('\n') {
        plain_text(row, &mut plain);
        let text = plain.trim_end();
        let Some(body) = text.strip_suffix('┃') else {
            continue;
        };
        if body.ends_with(' ')
            && !body.chars().any(|c| ('\u{2500}'..='\u{257f}').contains(&c))
            && row.rfind('┃').is_some_and(|at| isolated_scrollbar(row, at))
        {
            *counts.entry(text.width()).or_default() += 1;
        }
    }
    counts
        .into_iter()
        .rev()
        .find_map(|(width, count)| (count > 1).then_some(width))
}

/// Claude Code draws its input box rules at exactly the terminal width, which herdr's
/// `pane.layout` exceeds by 1 to 3 columns.
pub fn claude_rule_columns(ansi: &str) -> Option<usize> {
    let mut plain = String::new();
    ansi.split('\n')
        .filter_map(|row| {
            plain_text(row, &mut plain);
            let text = plain.trim_end();
            (!text.is_empty() && text.chars().all(|c| c == '─')).then(|| text.width())
        })
        .max()
}

fn isolated_scrollbar(row: &str, at: usize) -> bool {
    row[..at].rsplit_once("\u{1b}[").is_some_and(|(head, sgr)| {
        head.ends_with("\u{1b}[0m") && sgr.starts_with("38;") && sgr.ends_with('m')
    }) && row[at + '┃'.len_utf8()..].starts_with("\u{1b}[0m")
}

fn tui_wraps(kind: &str, ansi: &str, cols: usize) -> (Vec<u32>, Vec<u32>) {
    let markers: &[char] = if kind == "codex" {
        &['•', '›']
    } else {
        &['●']
    };
    let (mut wraps, mut splits) = (Vec::new(), Vec::new());
    let (mut plain, mut tail) = (String::new(), String::new());
    let mut prose = false;
    let mut base = 0;
    let mut prior_width = cols;
    let mut prev = Prev::Held;
    let mut box_top = false;
    let mut prompt: Option<SentPrompt> = None;
    for (i, row) in ansi.split('\n').enumerate() {
        plain_text(row, &mut plain);
        let trimmed = plain.trim_end();
        let scrollbar = kind == "copilot" && trimmed.ends_with('┃');
        let trimmed = if kind == "copilot" {
            trimmed.strip_suffix('┃').unwrap_or(trimmed).trim_end()
        } else {
            trimmed
        };
        let row_width = cols.saturating_sub(usize::from(scrollbar));
        let s = trimmed.trim_start_matches(' ');
        let ind = trimmed.len() - s.len();
        if kind == "copilot" {
            let below_top = std::mem::replace(
                &mut box_top,
                ind == 1 && !s.is_empty() && s.chars().all(|c| c == '▄'),
            );
            if let Some(sent) = prompt.as_mut().filter(|_| s.is_empty() || ind >= 4) {
                let text = trimmed.get(4..).unwrap_or_default();
                let at = u32::try_from(i).unwrap_or(u32::MAX);
                if sent.after(&tail, text) {
                    wraps.push(at);
                }
                sent.spaced = ends_in_space(row, false);
                row_above(&mut tail, text, 4);
                continue;
            }
            prompt = None;
            if let Some((text, width)) = sent_prompt(s).filter(|_| below_top && ind == 2) {
                row_above(&mut tail, text, 4);
                prompt = Some(SentPrompt {
                    width,
                    spaced: ends_in_space(row, true),
                    full: false,
                });
                continue;
            }
        }
        if s.is_empty() {
            prev = Prev::Start;
            prose = false;
            continue;
        }
        let marker_indent = if kind == "codex" { 0 } else { 1 };
        if let Some(body) = (ind == marker_indent)
            .then(|| s.strip_prefix(markers).and_then(|s| s.strip_prefix(' ')))
            .flatten()
        {
            base = ind + 2;
            prose = tui_prose(body, kind);
            prev = if prose {
                if let Some(len) = list_marker(body) {
                    let rest = body[len..].trim_start_matches(' ');
                    row_above(
                        &mut tail,
                        rest,
                        base + body[..body.len() - rest.len()].width(),
                    )
                } else {
                    row_above(&mut tail, body, base)
                }
            } else {
                Prev::Held
            };
            prior_width = row_width;
            continue;
        }
        if !prose && matches!(prev, Prev::Start) && ind == base {
            let body = list_marker(s)
                .map(|len| &s[len..])
                .unwrap_or(s)
                .trim_start();
            if body.chars().next().is_some_and(char::is_uppercase) && tui_prose(body, kind) {
                prose = true;
            }
        }
        if !prose || ind < base || tui_structure(s) {
            prose = false;
            prev = Prev::Held;
            continue;
        }
        if let Some(len) = list_marker(s) {
            let rest = s[len..].trim_start_matches(' ');
            prev = row_above(&mut tail, rest, ind + s[..s.len() - rest.len()].width());
            prior_width = row_width;
            continue;
        }
        let hang = match prev {
            Prev::Row(hang) => hang,
            Prev::Start => {
                prev = row_above(&mut tail, s, ind);
                prior_width = row_width;
                continue;
            }
            Prev::Held => continue,
        };
        if ind != hang {
            prev = Prev::Held;
            continue;
        }
        let available = prior_width.saturating_sub(hang);
        if tail.width() <= available {
            let at = u32::try_from(i).unwrap_or(u32::MAX);
            match separator(&tail, s, available) {
                " " if wide_cut(&tail, s, available) => splits.push(at),
                " " => wraps.push(at),
                "" => splits.push(at),
                _ => {}
            }
        }
        prev = row_above(&mut tail, s, ind);
        prior_width = row_width;
    }
    (wraps, splits)
}

fn tui_prose(body: &str, kind: &str) -> bool {
    let body = body.trim_start();
    body.chars().any(char::is_alphabetic)
        && !tui_structure(body)
        && match kind {
            "codex" => !["Ran ", "Explored", "Edited ", "Working ", "Failed "]
                .iter()
                .any(|prefix| body.starts_with(prefix)),
            "copilot" => !["Tip:", "MCP Servers", "Working ", "Thinking "]
                .iter()
                .any(|prefix| body.starts_with(prefix)),
            _ => false,
        }
}

fn tui_structure(text: &str) -> bool {
    text.contains("  ")
        || text.starts_with(['+', '$', '>', '|', '`'])
        || text.chars().any(|c| ('\u{2500}'..='\u{259f}').contains(&c))
}

/// The text of the first row of a sent Copilot prompt, `❯ ` then the text and an `HH:MM`
/// timestamp, and the width Copilot wrapped that text at: the columns before a space and
/// the timestamp.
fn sent_prompt(s: &str) -> Option<(&str, usize)> {
    let body = s.strip_prefix("❯ ")?;
    let (text, time) = body.split_at_checked(body.len().checked_sub(5)?)?;
    let time = time.as_bytes();
    let clock = time[2] == b':' && [0, 1, 3, 4].iter().all(|&i| time[i].is_ascii_digit());
    let text = text.strip_suffix(' ')?.trim_end();
    (clock && !text.is_empty()).then(|| (text, s.width() - 8))
}

/// A sent Copilot prompt, which Copilot wraps like wrap-ansi with `trim: false`: a row
/// broken after a word keeps that space at its end, and after a row the text fills, the
/// space goes, unseen, to the start of the next row.
struct SentPrompt {
    width: usize,
    /// The row above ends with one space in its text's style.
    spaced: bool,
    /// The last row checked filled the width.
    full: bool,
}

impl SentPrompt {
    /// Whether Copilot broke `tail` after a word before `next`; a typed newline could look
    /// like any other break, such as a full row cut inside a long token, or a row after a
    /// full row, which may start with an unseen space.
    fn after(&mut self, tail: &str, next: &str) -> bool {
        let used = tail.width();
        let after_full = std::mem::replace(&mut self.full, used >= self.width);
        let first = next.split(' ').next().unwrap_or_default().width();
        self.spaced
            && !after_full
            && !tail.is_empty()
            && !next.is_empty()
            && !next.starts_with(' ')
            && used < self.width
            && used + 1 + first > self.width
    }
}

/// Whether the text of a sent Copilot prompt's row ends with one space in its own style,
/// before the padding; the `first` row ends with a separately styled timestamp.
fn ends_in_space(row: &str, first: bool) -> bool {
    let mut marked = String::new();
    let mut chars = row.chars();
    while let Some(c) = chars.next() {
        match c {
            '\u{1b}' => {
                chars.by_ref().take_while(|&c| c != 'm').for_each(drop);
                marked.push('\0');
            }
            '\r' => {}
            c => marked.push(c),
        }
    }
    fn visible(s: &str) -> &str {
        s.trim_end_matches(|c: char| c.is_whitespace() || c == '\0')
    }
    let mut text = visible(&marked);
    if let Some(bar) = text.strip_suffix('┃') {
        text = visible(bar);
    }
    if first {
        let clock = text.len().saturating_sub(5);
        let Some(styled) = text.get(..clock).and_then(|t| t.strip_suffix('\0')) else {
            return false;
        };
        text = visible(styled);
    }
    let rest = &marked[text.len()..];
    rest.starts_with(' ') && !rest[1..].starts_with(' ')
}

/// Whether `row` sets a background before its first character.
fn shaded_start(row: &str) -> bool {
    let mut rest = row;
    while let Some((sgr, after)) = rest.strip_prefix("\u{1b}[").and_then(|r| r.split_once('m')) {
        if sgr.starts_with("48;") {
            return true;
        }
        rest = after;
    }
    false
}

fn row_above(tail: &mut String, text: &str, hang: usize) -> Prev {
    tail.clear();
    tail.push_str(text);
    Prev::Row(hang)
}

/// The characters `draft::cells` reads from `row`, without CR, into `out`.
fn plain_text(row: &str, out: &mut String) {
    out.clear();
    let mut chars = row.chars();
    while let Some(c) = chars.next() {
        match c {
            '\u{1b}' => chars
                .by_ref()
                .skip_while(|&c| c == '[')
                .take_while(|&c| c != 'm')
                .for_each(drop),
            '\r' => {}
            c => out.push(c),
        }
    }
}

/// A tool call such as `Bash(…)`, whose `⏺` blinks while it runs: joined only while the dot
/// shows, its rows would jump on the phone.
fn tool_header(s: &str) -> bool {
    let args = s.trim_start_matches(|c: char| c.is_ascii_alphanumeric() || c == '_');
    (args.len() < s.len() && args.starts_with('(')) || s.contains(" (MCP)(")
}

/// A run of wide characters cut where the next one no longer fits, one column short of `width`.
fn wide_cut(tail: &str, next: &str, width: usize) -> bool {
    let wide = |c: Option<char>| c.and_then(UnicodeWidthChar::width) == Some(2);
    wide(tail.chars().next_back()) && wide(next.chars().next()) && tail.width() + 2 > width
}

/// A [`wide_cut`] that wrap-ansi makes only inside a word longer than a row. Codex wraps with
/// textwrap, which also breaks between ideographs, so `tui_wraps` keeps [`wide_cut`].
fn wide_word_cut(tail: &str, next: &str, width: usize) -> bool {
    let last = tail.rsplit(' ').next().unwrap_or_default().width();
    let first = next.split(' ').next().unwrap_or_default().width();
    wide_cut(tail, next, width) && tail.width() + 1 == width && last + first > width
}

/// The byte length of a list marker such as `-`, `•` or `12.` that a space follows.
fn list_marker(s: &str) -> Option<usize> {
    let digits = s.bytes().take_while(u8::is_ascii_digit).count();
    let len = match s.chars().next()? {
        c @ ('-' | '*' | '•' | '+') => c.len_utf8(),
        _ if (1..=3).contains(&digits) && matches!(s.as_bytes().get(digits), Some(b'.' | b')')) => {
            digits + 1
        }
        _ => return None,
    };
    s[len..].starts_with(' ').then_some(len)
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;
    use crate::draft::model;

    fn joins(rows: &[&str], cols: usize) -> (Vec<u32>, Vec<u32>) {
        soft_wraps(&rows.join("\r\n"), cols)
    }

    fn wraps(rows: &[&str], cols: usize) -> Vec<u32> {
        let (wraps, splits) = joins(rows, cols);
        assert_eq!(splits, [] as [u32; 0]);
        wraps
    }

    /// The layout of a 189-column Claude Code pane in fullscreen mode, with its text replaced.
    const LIVE: &[&str] = &[
        "\u{1b}[0m\u{1b}[38;2;153;153;153m※\u{1b}[0m \u{1b}[0m\u{1b}[1m\u{1b}[38;2;153;153;153mrecap: \u{1b}[0m\u{1b}[3m\u{1b}[38;2;153;153;153mI've been extending the demo app's public API v12. Both branches are pushed, rebased on main, conflict-free, and the first branch's review fixes are tested end to end. Next is \u{1b}[0m",
        "  \u{1b}[0m\u{1b}[3m\u{1b}[38;2;153;153;153mwatching their CI, especially the Windows build.\u{1b}[0m",
        "     ",
        "\u{1b}[0m\u{1b}[38;2;255;255;255m⏺ \u{1b}[0mConfirmed the reviewer's first comment is wrong - 'nightly-image' is selected as intended since the release step only runs when a release is wanted, which never happens on a push. Now",
        "  checking which image the release job pushes to answer the second comment.",
        "",
        "\u{1b}[0m\u{1b}[38;2;255;255;255m⏺ \u{1b}[0m9 background shell command tasks didn't finish before the previous session ended. Task ids: b1aaaaaaa, b2bbbbbbb, b3ccccccc, b4ddddddd, b5eeeeeee, b6fffffff, b7ggggggg, b8hhhhhhh,",
        "b9iiiiiii.                                                                        ",
    ];

    #[test]
    fn live_prose_joins_at_the_mac_width() {
        assert_eq!(wraps(LIVE, 189), [1, 4]);
        assert_eq!(wraps(LIVE, 200), [] as [u32; 0]);
        assert_eq!(
            soft_wraps_for("claude", &LIVE.join("\r\n"), 189),
            joins(LIVE, 189)
        );
    }

    #[test]
    fn captured_korean_prose_joins_at_the_input_box_rule_width() {
        let screen = include_str!("../tests/fixtures/claude-2.1.303/korean-prose.ansi.txt");
        assert_eq!(claude_rule_columns(screen), Some(49));
        let status_rule = format!("{screen}\n{}", "─".repeat(20));
        assert_eq!(claude_rule_columns(&status_rule), Some(49));
        let rows: Vec<u32> = std::iter::once(8).chain(11..=26).collect();
        assert_eq!(soft_wraps(screen, 49), (rows.clone(), vec![]));
        let at_layout_width: Vec<u32> = rows
            .into_iter()
            .filter(|i| ![16, 20, 22, 26].contains(i))
            .collect();
        assert_eq!(soft_wraps(screen, 51), (at_layout_width, vec![]));
    }

    #[test]
    fn captured_codex_prose_joins_at_the_pane_width() {
        let screen = include_str!("../tests/fixtures/codex-2026-10-09/prose.ansi.txt");
        assert_eq!(soft_wraps_for("codex", screen, 186), (vec![1], vec![]));
        assert_eq!(soft_wraps_for("codex", screen, 200), (vec![], vec![]));
    }

    #[test]
    fn codex_prompt_wraps_join_without_changing_other_agents() {
        let captured = include_str!("../tests/fixtures/codex-0.160.1/working.ansi.txt");
        assert_eq!(soft_wraps_for("codex", captured, 212), (vec![31], vec![]));
        let rows = [
            "\u{1b}[48;2;51;51;60m› Please check the long prompt near its\u{1b}[0m",
            "\u{1b}[48;2;51;51;60m  end without adding a new line.\u{1b}[0m",
        ];
        let ansi = rows.join("\r\n");
        assert_eq!(soft_wraps_for("codex", &ansi, 40), (vec![1], vec![]));
        for kind in ["claude", "copilot"] {
            assert_eq!(soft_wraps_for(kind, &ansi, 40), (vec![], vec![]));
        }
        assert_eq!(soft_wraps_for("codex", &ansi, 80), (vec![], vec![]));
    }

    #[test]
    fn codex_prompt_explicit_newlines_and_code_stay_separate() {
        for ansi in [
            include_str!("../tests/fixtures/codex-0.160.1/draft-multiline.ansi.txt"),
            include_str!("../tests/fixtures/codex-0.160.1/draft-blank-line.ansi.txt"),
            include_str!("../tests/fixtures/codex-0.160.1/after-output.ansi.txt"),
        ] {
            assert_eq!(soft_wraps_for("codex", ansi, 212), (vec![], vec![]));
        }
        let rows = [
            "› Please check this example:",
            "  const longName = value",
            "    return longName",
            "",
            "  Keep this paragraph separate.",
        ];
        assert_eq!(
            soft_wraps_for("codex", &rows.join("\r\n"), 40),
            (vec![], vec![])
        );
    }

    #[test]
    fn codex_code_after_a_blank_and_nested_markers_keep_their_rows() {
        let rows = [
            "• Example:",
            "",
            "  const longName = value",
            "  return longName",
            "",
            "  • nested bullet outside a response",
            "    and its next line",
            "• A paragraph that fills",
            "  the remaining line.",
        ];
        assert_eq!(
            soft_wraps_for("codex", &rows.join("\r\n"), 24),
            (vec![8], vec![])
        );
    }

    #[test]
    fn codex_new_paragraph_after_a_blank_can_join() {
        let rows = [
            "• First paragraph ends.",
            "",
            "  Another paragraph fills",
            "  the remaining line.",
        ];
        assert_eq!(
            soft_wraps_for("codex", &rows.join("\r\n"), 25),
            (vec![3], vec![])
        );
    }

    #[test]
    fn codex_splits_between_ideographs_cut_short_of_the_edge() {
        let rows = [
            format!("• 运行 cargo tests {}", "之".repeat(10)),
            "  后检查结果。".to_owned(),
        ];
        assert_eq!(
            soft_wraps_for("codex", &rows.join("\r\n"), 40),
            (vec![], vec![1])
        );
    }

    #[test]
    fn codex_ui_code_and_diffs_keep_their_rows() {
        for screen in [
            include_str!("../tests/fixtures/codex-0.160.1/idle.ansi.txt"),
            include_str!("../tests/fixtures/codex-0.160.1/patch.ansi.txt"),
        ] {
            assert_eq!(soft_wraps_for("codex", screen, 212), (vec![], vec![]));
        }
    }

    #[test]
    fn copilot_prose_and_list_continuations_join() {
        let lines = [
            " ● The quick brown fox jumps over the  ┃",
            "   lazy dog keeps running.              ┃",
            "                                      ┃",
            " ● 1. The first list item goes all the  ┃",
            "      way to the next line.              ┃",
            "   2. A separate item.                  ┃",
        ];
        assert_eq!(
            soft_wraps_for("copilot", &lines.join("\r\n"), 40),
            (vec![1, 4], vec![])
        );
    }

    const SENT_WIDE: &str = include_str!("../tests/fixtures/copilot-1.0.96-2/sent-wide.ansi.txt");
    const SENT_NARROW: &str =
        include_str!("../tests/fixtures/copilot-1.0.96-2/sent-narrow.ansi.txt");
    const SENT_LINES: &str =
        include_str!("../tests/fixtures/copilot-1.0.96-2/sent-narrow-lines.ansi.txt");

    #[test]
    fn captured_copilot_sent_prompts_join_where_copilot_wrapped_them() {
        let cases = [
            (SENT_WIDE, 94, vec![2, 8, 17], vec![]),
            (SENT_NARROW, 50, vec![2, 3, 6, 11], vec![]),
            (
                SENT_LINES,
                50,
                vec![12, 13, 14, 17, 18, 23, 24, 25, 34],
                vec![],
            ),
        ];
        for (screen, cols, wraps, splits) in cases {
            assert_eq!(
                soft_wraps_for("copilot", screen, cols),
                (wraps.clone(), splits.clone())
            );
            assert_eq!(soft_wraps_for("copilot", screen, 212), (wraps, splits));
            for kind in ["claude", "codex"] {
                assert_eq!(soft_wraps_for(kind, screen, cols), (vec![], vec![]));
            }
        }
    }

    #[test]
    fn copilot_sent_prompt_reflow_keeps_one_timestamp_after_the_prompt() {
        let ansi = without_copilot_scrollbar(SENT_WIDE, 94);
        let (wraps, splits) = soft_wraps_for("copilot", SENT_WIDE, 94);
        let read = protocol::TerminalRead {
            terminal_id: protocol::TerminalId::new("t").unwrap(),
            source: protocol::ReadSource::Recent,
            ansi,
            truncated: false,
            wraps,
            splits,
            copilot_scrollbar: true,
        };
        let reflowed = read.reflowed().unwrap();
        assert_eq!(
            reflowed,
            include_str!("../tests/fixtures/copilot-1.0.96-2/sent-wide.reflowed.ansi.txt")
        );
        let mut plain = String::new();
        let lines: Vec<_> = reflowed
            .split('\n')
            .map(|row| {
                plain_text(row, &mut plain);
                plain.trim().to_owned()
            })
            .collect();
        assert!(lines.contains(&"❯ Without using any tools, reply with just the word OK. This sentence is only padding so that the prompt runs across several rows of the terminal and wraps at 10:19".into()));
        assert!(lines.contains(&"the pane width, which is what this capture checks.".into()));
        assert!(lines.contains(&format!(
            "❯ Without using any tools, reply with just OK. https://example.com/{} 10:20",
            "abcdefghij".repeat(6)
        )));
        assert!(lines.contains(&"abcdefghij".repeat(8)));
        assert!(lines.contains(
            &"to wrap across rows at the pane width.  Indented line after a blank one.- a".into()
        ));
        assert!(lines.contains(&"list item that is long enough to wrap across rows at this pane width too.Short.let value = compute(alpha, beta);".into()));
        for line in [
            "Indented line after a blank one.",
            "- a list item that is long enough to wrap across rows at this pane width too.",
            "Short.",
            "next line after a full one.",
        ] {
            assert!(lines.contains(&line.into()), "{line}");
        }
        for time in ["10:19", "10:20", "10:28", "10:29"] {
            assert_eq!(reflowed.matches(time).count(), 1);
        }
    }

    fn sent(rows: &[&str], width: usize) -> String {
        let shade = "\u{1b}[48;2;20;27;34m";
        let text = "\u{1b}[38;2;240;246;252m\u{1b}[48;2;20;27;34m";
        let border = |block: &str| {
            format!(
                " \u{1b}[0m\u{1b}[38;2;20;27;34m{}\u{1b}[0m",
                block.repeat(width + 11)
            )
        };
        let mut out = vec![border("▄")];
        for (i, row) in rows.iter().enumerate() {
            let (lead, tail) = if i == 0 {
                (
                    " \u{1b}[0m",
                    format!(
                        " \u{1b}[0m\u{1b}[38;2;145;152;161m{shade}12:34\u{1b}[0m{shade}  \u{1b}[0m"
                    ),
                )
            } else {
                ("   \u{1b}[0m", format!("      \u{1b}[0m{shade}  \u{1b}[0m"))
            };
            let marker = if i == 0 { "❯ " } else { "" };
            let pad = " ".repeat(width - row.width());
            out.push(format!(
                " \u{1b}[0m{shade}{lead}{text}{marker}{row}\u{1b}[0m{shade}{pad}{tail}"
            ));
        }
        out.push(border("▀"));
        out.join("\r\n")
    }

    #[test]
    fn copilot_sent_prompts_join_only_breaks_copilot_certainly_made() {
        let wrapped = sent(&["The quick brown fox ", "jumps over it."], 20);
        assert_eq!(soft_wraps_for("copilot", &wrapped, 34), (vec![2], vec![]));
        let wide = "文字".repeat(9);
        for screen in [
            sent(&[&wide, "文字"], 37),
            sent(&["Paths:", "/usr/local/bin/tool1", "Another line."], 20),
            sent(
                &["Paths:", "/usr/local/bin/tool1", "aaaa bbbb cccc ", "ddddd"],
                20,
            ),
            sent(
                &[
                    "A long path here:",
                    "/usr/local/bin/tool1",
                    "/usr/local/lib",
                ],
                20,
            ),
            sent(&["Two spaces end this  ", "jumps over it."], 21),
            sent(&["The quick brown fox", "jumps over it."], 20),
            sent(&["The quick brown fox ", " jumps over it."], 20),
            sent(&["The quick brown fox ", "", "jumps over it."], 20),
            wrapped.replacen('▄', "─", 1),
            wrapped.replace("12:34", "12 34"),
            wrapped.split_once("\r\n").unwrap().1.to_owned(),
        ] {
            assert_eq!(
                soft_wraps_for("copilot", &screen, 34),
                (vec![], vec![]),
                "{screen:?}"
            );
        }
    }

    #[test]
    fn copilot_sent_prompt_rows_ending_in_multibyte_characters_join_nothing() {
        for pad in ["\u{a0}", "\u{3000}", "\u{85}", "界", "🐕"] {
            let rows = format!(" ▄▄▄▄▄▄\r\n  ❯ hello 10:19{}\r\n ▀▀▀▀", pad.repeat(3));
            assert_eq!(soft_wraps_for("copilot", &rows, 34), (vec![], vec![]));
            let rows = format!(
                " ▄▄▄▄▄▄\r\n  ❯ hello 10:19{}\r\n    world\r\n ▀▀▀▀",
                pad.repeat(3)
            );
            assert_eq!(soft_wraps_for("copilot", &rows, 34), (vec![], vec![]));
        }
        assert_eq!(
            soft_wraps_for(
                "copilot",
                " ▄▄\r\n  ❯ a\u{1b}[0m \u{1b}[38;5m10:19\u{1b}[\r\n    b",
                34
            ),
            (vec![], vec![])
        );
    }

    #[test]
    fn copilot_scrollbar_is_removed_from_all_rows_without_changing_sgr() {
        let screen = include_str!("../tests/fixtures/copilot-1.0.93-1/output-long.ansi.txt");
        let cleaned = without_copilot_scrollbar(screen, 212);
        assert!(cleaned.matches('┃').count() < screen.matches('┃').count());
        let mut plain = String::new();
        for row in cleaned.lines() {
            plain_text(row, &mut plain);
            assert!(!(plain.trim_end().ends_with('┃') && plain.trim_end().width() == 212));
        }
        let sgr = |text: &str| {
            text.split('\u{1b}')
                .skip(1)
                .map(|part| part.split_once('m').unwrap().0.to_owned())
                .collect::<Vec<_>>()
        };
        assert_eq!(sgr(&cleaned), sgr(screen));
        assert!(cleaned.contains("Without using any tools"));
        assert!(cleaned.contains("18:31"));
    }

    #[test]
    fn copilot_scrollbar_cleanup_keeps_borders_and_bars_inside_content() {
        for row in ["│ a │", "┃ a ┃", "word┃", "a ┃ b", "a┃"] {
            assert_eq!(without_copilot_scrollbar(row, 5), row);
        }
        let gray = "\u{1b}[0m\u{1b}[38;2;145;152;161m";
        for text in ["└ ┃ text ┃", "└ ─────┘ ┃"] {
            let plain = format!("   {text}");
            assert_eq!(without_copilot_scrollbar(&plain, plain.width()), plain);
            let styled = format!("   {gray}{text}\u{1b}[0m");
            assert_eq!(without_copilot_scrollbar(&styled, plain.width()), styled);
        }
        let literal = "   └ literal | pipe ┃";
        assert_eq!(without_copilot_scrollbar(literal, literal.width()), literal);
        assert_eq!(without_copilot_scrollbar("wide界 ┃", 8), "wide界 ");
        assert_eq!(
            without_copilot_scrollbar("\u{1b}[48;2;20;27;34mabc \u{1b}[0m┃\u{1b}[0m\r", 5),
            "\u{1b}[48;2;20;27;34mabc \u{1b}[0m\u{1b}[0m\r"
        );
        assert_eq!(without_copilot_scrollbar("abc ┃", 0), "abc ┃");
        let bar = "\u{1b}[0m\u{1b}[38;2;145;152;161m┃\u{1b}[0m";
        assert_eq!(
            without_copilot_scrollbar(&format!("│ a │ {bar}"), 7),
            "│ a │ \u{1b}[0m\u{1b}[38;2;145;152;161m\u{1b}[0m"
        );
    }

    #[test]
    fn only_repeated_isolated_scrollbars_supply_a_missing_pane_width() {
        let screen = include_str!("../tests/fixtures/copilot-1.0.93-1/output-long.ansi.txt");
        assert_eq!(copilot_scrollbar_columns(screen), Some(212));
        assert_eq!(copilot_scrollbar_columns("abc ┃\nxyz ┃"), None);
        let bar = "\u{1b}[0m\u{1b}[38;2;145;152;161m┃\u{1b}[0m";
        assert_eq!(copilot_scrollbar_columns(&format!("abc {bar}")), None);
        assert_eq!(
            copilot_scrollbar_columns(&format!("┃ a {bar}\n┃ b {bar}")),
            None
        );
    }

    #[test]
    fn captured_copilot_ui_and_numbered_rows_stay_separate() {
        for screen in [
            include_str!("../tests/fixtures/copilot-1.0.93-1/output-long.ansi.txt"),
            include_str!("../tests/fixtures/copilot-1.0.94-1/narrower-output.ansi.txt"),
            include_str!("../tests/fixtures/copilot-1.0.93-1/permission.ansi.txt"),
        ] {
            assert_eq!(soft_wraps_for("copilot", screen, 212), (vec![], vec![]));
        }
    }

    #[test]
    fn prose_continuations_join() {
        let rows = [
            "⏺ The quick brown fox jumps over the",
            "  lazy dog and then it runs off to the",
            "  woods.",
            "",
            "  aaaa bbbb cccc dddd eeee ffff gggg hhh",
            "  iiii.",
            "  - an item that wraps onto the next one",
            "    row at the hang of its marker.",
            "  12. a numbered item wraps onto the",
            "      next row past its number.",
            "     • a nested item that wraps onto the",
            "       row below it.",
            "",
            "  A row that ends with the word per step",
            "  2.",
        ];
        assert_eq!(wraps(&rows, 40), [1, 2, 5, 7, 9, 11, 14]);
    }

    #[test]
    fn claude_sent_prompts_join_only_where_claude_wrapped_them() {
        let wrapped = include_str!("../tests/fixtures/claude-2.1.293/heads-up.ansi.txt");
        assert_eq!(soft_wraps(wrapped, 212), (vec![6, 14], vec![]));
        let (wraps, splits) = soft_wraps(wrapped, 212);
        let read = protocol::TerminalRead {
            terminal_id: protocol::TerminalId::new("t").unwrap(),
            source: protocol::ReadSource::Recent,
            ansi: wrapped.into(),
            truncated: false,
            wraps,
            splits,
            copilot_scrollbar: false,
        };
        let joined = read.reflowed().unwrap();
        assert!(joined.contains("6) run python3 -c \u{1b}[0m\u{1b}[48;2;55;55;55m\u{1b}[0m\u{1b}[0m\u{1b}[48;2;55;55;55m\u{1b}[0m\u{1b}[38;2;255;255;255m\u{1b}[48;2;55;55;55m'print(round"));
        let typed = include_str!("../tests/fixtures/claude-2.1.299/sent-multiline.ansi.txt");
        assert_eq!(soft_wraps(typed, 182), (vec![8], vec![]));
    }

    #[test]
    fn claude_logo_loses_only_its_true_black_background() {
        let screen = include_str!("../tests/fixtures/claude-2.1.293/heads-up.ansi.txt");
        let cleaned = without_claude_black(screen);
        assert!(screen.contains("\u{1b}[48;2;0;0;0m▛███▛█   \u{1b}[0m"));
        assert!(cleaned.contains("\u{1b}[49m▛███▛█   \u{1b}[0m"));
        assert!(!cleaned.contains("48;2;0;0;0"));
        let combined = "\u{1b}[1;48;2;0;0;0mx\u{1b}[38;2;0;0;0mx";
        assert_eq!(without_claude_black(combined), combined);
    }

    #[test]
    fn a_word_longer_than_a_row_splits() {
        let long = format!("⏺ {}", "x".repeat(38));
        let rows = [long.as_str(), "  yyyy for the details."];
        assert_eq!(joins(&rows, 40), (vec![], vec![1]));
    }

    #[test]
    fn text_after_a_table_joins() {
        let rows = [
            "⏺ The results are in the table below, by",
            "  ┌──────┬──────┐",
            "  │ a    │ b    │",
            "  └──────┴──────┘",
            "  and text after the table goes onto the",
            "  next row.",
        ];
        assert_eq!(wraps(&rows, 40), [5]);
    }

    #[test]
    fn real_line_ends_stay() {
        let rows = [
            "  indented rows before any message that",
            "  would otherwise join.",
            "⏺ A short row.",
            "  Next row.",
            "  A row before a blank row that is long",
            "",
            "  and the next paragraph.",
            "  - an item that ends at the very edge.",
            "  - Next item.",
            "  * a Jira item that runs on and on to the",
            "  next row which also runs onto the next",
            "  row.",
            "  │ a table row that is wide enough here",
            "  │ and another one that is wide as well",
            "⏺ Bash(cd /src/app && cargo test --all",
            "  --quiet)",
            "  ⎿  ok",
            "",
            "⏺ A reply between two tool calls, and the",
            "",
            "  Bash(cd /src/app && cargo test --all",
            "  --quiet)",
            "  ⎿  ok",
            "",
            "⏺ A reply between two tool calls, and the",
            "",
            "  slack - post_message (MCP)(channel: \"a",
            "  \")",
            "",
            "⏺ Update(src/main.rs)",
            "  ⎿  Updated src/main.rs with 1 addition",
            "     165: let value = compute(alpha, beta",
            "     )'))",
            "       12 -  let old = compute(alpha, beta,",
            "       12 +  let new = compute(alpha, beta,",
            "⏺ Task ids: b2djyl00u, bfbmarhm3, b5050hg8",
            "bmlpyo7gu.",
            "────────────────────────────────────────",
            "❯ a draft typed into the input box that",
            "  wraps onto a second row of the input",
            "────────────────────────────────────────",
            "  Opus 5.5 high · ~/src/app on main with",
            "  ⏵⏵ auto mode on (shift+tab to cycle)",
        ];
        assert_eq!(joins(&rows, 40), (vec![], vec![]));
    }

    #[test]
    fn rows_wider_than_the_pane_never_join() {
        let wide = format!("⏺ {}", "word ".repeat(10));
        let rows = [wide.as_str(), "  next"];
        assert_eq!(joins(&rows, 40), (vec![], vec![]));
        let rows = ["⏺ abcd efgh ijkl mnop qrst uvwx yzab cdef", "  next"];
        assert_eq!(joins(&rows, 40), (vec![], vec![]));
        assert_eq!(joins(LIVE, 0), (vec![], vec![]));
    }

    #[test]
    fn a_blank_row_ends_a_held_block() {
        let rows = [
            "⏺ Notes:",
            "  * a Jira item that runs on and on to the",
            "  next row which also runs onto the next",
            "  row.",
            "",
            "  A paragraph after the list that wraps",
            "  at the edge.",
        ];
        assert_eq!(wraps(&rows, 40), [6]);
    }

    #[test]
    fn rows_after_a_full_row_hold_a_column_less() {
        // Claude Code 2.1.302 replied with these at 46 columns.
        let row = |mark: &str, text: &str| format!("{mark} {text}");
        let (a, b, c, d) = (
            "a".repeat(20),
            "b".repeat(25),
            "c".repeat(20),
            "d".repeat(25),
        );
        let rows = [row("⏺", &format!("{a} {b}")), row(" ", &c), row(" ", &d)];
        assert_eq!(
            joins(&rows.each_ref().map(String::as_str), 48),
            (vec![1, 2], vec![])
        );
        for (first, next) in [
            (
                "한국어 한국어 한국어 한국어 한국어 문장 한국어",
                "다음 문장",
            ),
            ("한국어 한국어 한국어 한국어 한국어 대한민국어", "다음"),
        ] {
            let rows = [row("⏺", first), row(" ", next)];
            assert_eq!(
                joins(&rows.each_ref().map(String::as_str), 48),
                (vec![1], vec![])
            );
        }
    }

    #[test]
    fn a_full_row_does_not_read_ahead_into_a_list() {
        let url = format!("https://example.com/{}", "a".repeat(26));
        let rows = [
            "⏺ Run this:".to_owned(),
            format!("  {url}"),
            "  - item".to_owned(),
        ];
        assert_eq!(
            joins(&rows.each_ref().map(String::as_str), 48),
            (vec![1], vec![])
        );
    }

    #[test]
    fn wide_characters_cut_short_of_the_edge_split() {
        let row = format!("⏺ {}", "中".repeat(93));
        let rows = [row.as_str(), "  文字。"];
        assert_eq!(joins(&rows, 189), (vec![], vec![1]));
        assert_eq!(joins(&rows, 190), (vec![1], vec![]));
    }

    proptest! {
        #[test]
        fn prose_joins_where_it_wrapped(
            (paragraphs, cols) in (prop::collection::vec(model::text(), 1..3), 6..26usize)
                .prop_filter("unreadable", |(paragraphs, cols)| {
                    // The readers do not follow a word over a row a run of wide characters
                    // left one column short, and a blank row (only the hidden space after a
                    // full row) ends a paragraph.
                    paragraphs.iter().all(|p| {
                        let rows = model::rows(p, cols - 2);
                        !rows
                            .windows(2)
                            .any(|r| r[1].0.is_empty() && r[0].1.trim_end().width() < cols - 2)
                            && !rows.iter().any(|(sep, r)| *sep == " " && r.is_empty())
                    })
                }),
        ) {
            let width = cols - 2;
            let (mut screen, mut drawn) = (Vec::new(), Vec::new());
            for paragraph in &paragraphs {
                let rows = model::rows(paragraph, width);
                for (k, (_, row)) in rows.iter().enumerate() {
                    let mark = if k == 0 { "⏺" } else { " " };
                    screen.push(format!("{mark} {row}"));
                }
                drawn.push((screen.len() - rows.len(), paragraph, rows));
                screen.push(String::new());
            }
            let (wraps, splits) = soft_wraps(&screen.join("\n"), cols);
            let mut joined = 0;
            for (at, paragraph, rows) in drawn {
                let mut read = String::new();
                for (k, (_, row)) in rows.iter().enumerate() {
                    let i = u32::try_from(at + k).unwrap();
                    if k > 0 {
                        joined += usize::from(wraps.contains(&i) || splits.contains(&i));
                        read.push_str(if wraps.contains(&i) {
                            " "
                        } else if splits.contains(&i) {
                            ""
                        } else {
                            "\n"
                        });
                    }
                    read.push_str(row.trim_end());
                }
                prop_assert!(
                    read == *paragraph || model::reads_first(paragraph, &read, width),
                    "{:?} read as {:?}",
                    paragraph,
                    read
                );
            }
            prop_assert_eq!(joined, wraps.len() + splits.len());
        }

        #[test]
        fn readers_never_panic(text in crate::prompt::fixtures::screen(), cols in 0..120usize) {
            let rows = text.split('\n').count();
            for kind in ["claude", "codex", "copilot"] {
                let (wraps, splits) = soft_wraps_for(kind, &text, cols);
                for at in [wraps, splits] {
                    prop_assert!(at.is_sorted() && at.iter().all(|&i| (i as usize) < rows));
                }
            }
            prop_assert_eq!(without_copilot_scrollbar(&text, cols).split('\n').count(), rows);
            copilot_scrollbar_columns(&text);
        }
    }

    #[test]
    fn plain_text_reads_what_draft_reads() {
        let mut screens = vec![LIVE.join("\r\n")];
        let mut dirs =
            vec![std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")];
        while let Some(dir) = dirs.pop() {
            for entry in std::fs::read_dir(dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    dirs.push(path);
                } else {
                    let raw = String::from_utf8_lossy(&std::fs::read(&path).unwrap()).into_owned();
                    screens.push(crate::drive::sanitize_ansi(&raw));
                    screens.push(raw);
                }
            }
        }
        screens.push("a\u{1b}[2;38;5;1mb\u{1b}[\u{1b}c\r\u{1b}".into());
        let mut plain = String::new();
        for row in screens.iter().flat_map(|s| s.split('\n')) {
            plain_text(row, &mut plain);
            let cells: String = crate::draft::cells(row)
                .into_iter()
                .map(|(c, _)| c)
                .filter(|&c| c != '\r')
                .collect();
            assert_eq!(plain, cells, "{row:?}");
        }
    }
}
