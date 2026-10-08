use protocol::limits::MAX_PROMPT_BYTES;
use unicode_width::UnicodeWidthStr;

use crate::prompt;

/// Claude Code's input box as drawn on the visible screen.
#[derive(Debug, Clone, PartialEq)]
pub enum InputBox {
    Draft(Draft),
    /// Holds what a screen read cannot carry back and the phone must never replace: a
    /// collapsed paste or image, or a mode other than `❯`, such as bash mode's `!`.
    Opaque,
}

/// Unsent text in the input box.
#[derive(Debug, Clone, PartialEq)]
pub struct Draft {
    /// Normalized; empty when the box shows only its placeholder.
    pub text: String,
    /// Visual rows of the box, wrapped rows included: what the clearing keys walk.
    pub lines: usize,
}

/// Claude Code 2.1 shows these in place of content kept outside the input text; its own
/// pattern is `\[(?:Pasted text|Image|\.\.\.Truncated text|✦ Team setup guide) #\d+...\]`
/// plus `\[⧉ ...\]`. Clearing one deletes content the phone never saw.
const COLLAPSED: [&str; 5] = [
    "[Pasted text #",
    "[Image #",
    "[...Truncated text #",
    "[\u{2726} Team setup guide #",
    "[\u{29c9} ",
];

/// The input box is the last block on screen between two horizontal rules drawn from
/// column 0, whose first row starts at column 0 and whose other rows are indented by two
/// spaces, so a draft row of `─` never closes it. Dim text inside it is Claude Code's own
/// placeholder or hint, never typed text. `None` when the screen shows no such box, as
/// while a dialog replaces it or once a draft taller than the screen pushed its top rule
/// off.
pub fn parse(ansi: &str) -> Option<InputBox> {
    let rows: Vec<Vec<(char, bool)>> = ansi.split('\n').map(cells).collect();
    let plain: Vec<String> = rows
        .iter()
        .map(|r| {
            r.iter()
                .map(|(c, _)| *c)
                .collect::<String>()
                .trim_end()
                .to_owned()
        })
        .collect();
    let rules: Vec<usize> = (0..plain.len())
        .filter(|&i| plain[i].starts_with('─') && prompt::is_rule(&plain[i]))
        .collect();
    let (top, bottom) = rules
        .windows(2)
        .rev()
        .map(|w| (w[0], w[1]))
        .find(|&(t, b)| {
            b > t + 1
                && !plain[t + 1].is_empty()
                && !plain[t + 1].starts_with(char::is_whitespace)
                && plain[t + 2..b]
                    .iter()
                    .all(|l| l.is_empty() || l.starts_with("  "))
        })?;
    let shown: String = plain[top + 1..bottom].join(" ");
    let shown = shown.split_whitespace().collect::<Vec<_>>().join(" ");
    if !plain[top + 1].starts_with('❯') || COLLAPSED.iter().any(|p| shown.contains(p)) {
        return Some(InputBox::Opaque);
    }
    let lines: Vec<String> = rows[top + 1..bottom]
        .iter()
        .enumerate()
        .map(|(i, row)| {
            let skip = if i == 0 {
                1 + usize::from(row.get(1).is_some_and(|(c, _)| matches!(c, ' ' | '\u{a0}')))
            } else {
                2
            };
            row.iter()
                .skip(skip)
                .filter(|(c, dim)| !dim && !c.is_control() && !protocol::is_format(*c))
                .map(|(c, _)| *c)
                .collect::<String>()
                .trim_end()
                .to_owned()
        })
        .collect();
    let width = plain[bottom].width().saturating_sub(4);
    let mut text = String::new();
    for (i, line) in lines.iter().enumerate() {
        if i > 0 {
            text.push_str(separator(&lines[i - 1], line, width));
        }
        text.push_str(line);
    }
    let mut text = normalize(&text);
    if text.len() > MAX_PROMPT_BYTES {
        let mut end = MAX_PROMPT_BYTES;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text = normalize(&text[..end]);
    }
    Some(InputBox::Draft(Draft {
        text,
        lines: lines.len(),
    }))
}

/// Claude Code 2.1 wraps the draft with wrap-ansi `{hard: true, trim: false}` at the
/// terminal width (the rule's) minus 4 and hides the spaces around each soft wrap. A soft
/// wrap is a row the next word did not fit on, or a full row that split a word longer
/// than a row; a row starting with a space follows a typed newline. A typed newline right
/// where the next word would not have fit reads as a space.
pub(crate) fn separator(prev: &str, next: &str, width: usize) -> &'static str {
    if width == 0 || prev.is_empty() || next.is_empty() || next.starts_with(char::is_whitespace) {
        return "\n";
    }
    let first = next.split(' ').next().unwrap_or_default().width();
    let used = prev.width();
    if used >= width {
        let last = prev.rsplit(' ').next().unwrap_or_default().width();
        if last + first > width { "" } else { " " }
    } else if used + 1 + first > width {
        " "
    } else {
        "\n"
    }
}

/// Trailing whitespace is not something a screen read can tell apart from padding.
pub fn normalize(text: &str) -> String {
    text.lines()
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n")
        .trim_end()
        .to_owned()
}

/// Verified on Claude Code 2.x in herdr 0.9.3 for a draft of `lines` rows with the cursor
/// anywhere: `down` to the last row (a no-op there), then per row `ctrl+e`, `ctrl+u`
/// (clear it) and `backspace` (join it to the row above; a no-op on an empty first row).
/// Never `ctrl+c`, which interrupts a working agent, nor `ctrl+d`, which exits Claude
/// Code on an empty input.
pub fn clear_keys(lines: usize) -> Vec<&'static str> {
    let mut keys = vec!["down"; lines.saturating_sub(1)];
    for _ in 0..lines {
        keys.extend(["ctrl+e", "ctrl+u", "backspace"]);
    }
    keys
}

/// Each character with whether SGR dim (2) is on. The input went through
/// `drive::sanitize_ansi`, so every ESC starts a plain SGR.
pub(crate) fn cells(row: &str) -> Vec<(char, bool)> {
    let mut out = Vec::new();
    let mut dim = false;
    let mut chars = row.chars();
    while let Some(c) = chars.next() {
        if c != '\u{1b}' {
            out.push((c, dim));
            continue;
        }
        let params: String = chars
            .by_ref()
            .skip_while(|&c| c == '[')
            .take_while(|&c| c != 'm')
            .collect();
        if params.is_empty() {
            dim = false;
        }
        let mut it = params.split(';');
        while let Some(p) = it.next() {
            match p {
                "" | "0" | "22" => dim = false,
                "2" => dim = true,
                "38" | "48" | "58" => match it.next() {
                    Some("5") => {
                        it.next();
                    }
                    Some("2") => {
                        it.nth(2);
                    }
                    _ => {}
                },
                _ => {}
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::drive::sanitize_ansi;

    const RULE: &str = "\u{1b}[38;2;136;136;136m────────────────────────────────────────\u{1b}[39m";
    const LABELED: &str =
        "\u{1b}[38;2;136;136;136m──────────────────────────── ultracode ─\u{1b}[39m";
    const STATUS: &str = "  \u{1b}[38;2;153;153;153mOpus 5.5 high\u{1b}[39m · ~/src/app\n  \u{1b}[38;2;215;119;87m⏵⏵ auto mode on\u{1b}[39m \u{1b}[2m(shift+tab to cycle)\u{1b}[0m\n";
    const PLACEHOLDER: &str =
        "❯ \u{1b}[0m\u{1b}[2mTry \"create a util logging.py that...\"\u{1b}[0m";

    fn screen(top: &str, box_rows: &str) -> String {
        format!("⏺ Done. The build passes.\r\n\r\n{top}\r\n{box_rows}\r\n{RULE}\r\n{STATUS}")
    }

    fn read(s: &str) -> Option<InputBox> {
        parse(&sanitize_ansi(s))
    }

    fn draft(text: &str, lines: usize) -> Option<InputBox> {
        Some(InputBox::Draft(Draft {
            text: text.into(),
            lines,
        }))
    }

    #[test]
    fn placeholder_and_bare_prompt_are_empty() {
        assert_eq!(read(&screen(RULE, PLACEHOLDER)), draft("", 1));
        assert_eq!(read(&screen(RULE, "❯\u{a0}")), draft("", 1));
        assert_eq!(read(&screen(RULE, "❯")), draft("", 1));
        assert_eq!(read(&screen(RULE, "❯ ")), draft("", 1));
    }

    #[test]
    fn typed_text() {
        assert_eq!(
            read(&screen(RULE, "❯\u{a0}hello world")),
            draft("hello world", 1)
        );
        assert_eq!(
            read(&screen(RULE, "❯ one\n  two\n  three")),
            draft("one\ntwo\nthree", 3)
        );
        assert_eq!(
            read(&screen(
                LABELED,
                "❯ \u{1b}[1mone\u{1b}[22m  \n    indented\n  \n  last"
            )),
            draft("one\n  indented\n\nlast", 4)
        );
        assert_eq!(
            read(&screen(RULE, "❯ /he\u{1b}[2mlp\u{1b}[22m")),
            draft("/he", 1),
            "dim completion hint is not typed text"
        );
        assert_eq!(
            read(&screen(RULE, "❯ two  spaces\u{202e}x\t")),
            draft("two  spacesx", 1)
        );
        assert_eq!(
            read(&screen(RULE, "❯ one\n  ────────────\n  two")),
            draft("one\n────────────\ntwo", 3),
            "a typed rule is indented, so it does not close the box"
        );
    }

    #[test]
    fn soft_wraps_are_not_newlines() {
        // RULE is 40 columns, so a row holds 36.
        assert_eq!(
            read(&screen(
                RULE,
                "❯ the quick brown fox jumps over the\n  lazy dog"
            )),
            draft("the quick brown fox jumps over the lazy dog", 2)
        );
        let long = "x".repeat(36);
        assert_eq!(
            read(&screen(RULE, &format!("❯ {long}\n  {long}\n  yy end"))),
            draft(&format!("{long}{long}yy end"), 3),
            "a word longer than a row is split mid-word"
        );
        assert_eq!(
            read(&screen(
                RULE,
                "❯ aaaa bbbb cccc dddd eeee ffff gggg h\n  next"
            )),
            draft("aaaa bbbb cccc dddd eeee ffff gggg h next", 2),
            "a full row that ended on a word"
        );
        assert_eq!(
            read(&screen(RULE, "❯ 日本語のテキストを入力しています\n  です")),
            draft("日本語のテキストを入力しています です", 2),
            "wide characters count two columns"
        );
        assert_eq!(
            read(&screen(RULE, "❯ short line\n  next\n\n  after a blank")),
            draft("short line\nnext\n\nafter a blank", 4)
        );
        assert_eq!(
            read(&screen(
                RULE,
                "❯ the quick brown fox jumps over the\n    indented"
            )),
            draft("the quick brown fox jumps over the\n  indented", 2),
            "a row starting with a space follows a typed newline"
        );
    }

    #[test]
    fn slash_command_screens() {
        for (ansi, text) in [
            (
                include_str!("../tests/fixtures/claude-2.1.293/slash-menu.ansi.txt"),
                "/s",
            ),
            (
                include_str!("../tests/fixtures/claude-2.1.293/slash-menu-down.ansi.txt"),
                "/s",
            ),
            (
                include_str!("../tests/fixtures/claude-2.1.293/slash-tab.ansi.txt"),
                "/skills",
            ),
            (
                include_str!("../tests/fixtures/claude-2.1.293/slash-no-match.ansi.txt"),
                "/zzq",
            ),
            (
                include_str!("../tests/fixtures/claude-2.1.293/slash-tab-hint.ansi.txt"),
                "/rename  [name]",
            ),
            (
                include_str!("../tests/fixtures/claude-2.1.293/slash-sta.ansi.txt"),
                "/sta",
            ),
            (
                include_str!("../tests/fixtures/claude-2.1.293/slash-stat.ansi.txt"),
                "/stat",
            ),
            (
                include_str!("../tests/fixtures/claude-2.1.293/slash-statu.ansi.txt"),
                "/statu",
            ),
            (
                include_str!("../tests/fixtures/claude-2.1.293/slash-statstatu.ansi.txt"),
                "/stat/statu",
            ),
        ] {
            assert_eq!(read(ansi), draft(text, 1), "{text}");
        }
    }

    #[test]
    fn collapsed_content_and_other_modes_are_opaque() {
        for rows in [
            "❯ fix this [Pasted text #1 +40 lines]",
            "❯ [Image #1] what is this?",
            "❯ see \u{1b}[2m[Pasted text #2]\u{1b}[22m",
            "❯ the quick brown fox jumps over [Pasted\n  text #1 +3 lines]",
            "❯ [...Truncated text #1 +200 lines...]",
            "! git push",
            "!",
        ] {
            assert_eq!(read(&screen(RULE, rows)), Some(InputBox::Opaque), "{rows}");
        }
    }

    #[test]
    fn no_box_is_unknown() {
        let dialog = format!(
            "⏺ Bash(rm -rf build)\n  ⎿  Running…\n\n{}",
            prompt::fixtures::BASH
        );
        assert_eq!(read(&dialog), None);
        assert_eq!(read(prompt::fixtures::TRUST), None);
        assert_eq!(read(""), None);
        assert_eq!(read("plain shell output\n$ "), None);
        assert_eq!(read(&format!("{RULE}\n❯ open box")), None);
        assert_eq!(read(&format!("❯ no top rule\n{RULE}\n")), None);
        assert_eq!(read(&format!("{RULE}\n❯ one\nunindented\n{RULE}\n")), None);
        assert_eq!(
            read(&format!(
                "  rows of a draft\n  taller than the screen\n{RULE}\n{STATUS}"
            )),
            None
        );
        assert_eq!(
            read(&format!("{RULE}\nnot a prompt\n{RULE}\n")),
            Some(InputBox::Opaque)
        );
    }

    #[test]
    fn output_above_the_box_is_not_the_draft() {
        let echoed = format!(
            "{RULE}\n❯ an old prompt\n{RULE}\n⏺ Answer\n  ❯ quoted in output\n\n{}",
            screen(RULE, PLACEHOLDER)
        );
        assert_eq!(read(&echoed), draft("", 1));
        let typed = format!("{RULE}\n❯ an old prompt\n{RULE}\n{}", screen(RULE, "❯ new"));
        assert_eq!(read(&typed), draft("new", 1));
        let ruled_status = format!("{}{RULE}\n  more status\n{RULE}\n", screen(RULE, "❯ new"));
        assert_eq!(read(&ruled_status), draft("new", 1));
    }

    #[test]
    fn long_drafts_are_capped() {
        let long = format!("❯ {}é", "a".repeat(MAX_PROMPT_BYTES - 1));
        let Some(InputBox::Draft(d)) = read(&screen(RULE, &long)) else {
            panic!("no draft");
        };
        assert_eq!(d.text.len(), MAX_PROMPT_BYTES - 1);
        assert!(protocol::DraftText::new(d.text).is_ok());
    }

    #[test]
    fn normalizes_trailing_whitespace() {
        assert_eq!(normalize("one  \ntwo\t\n\n  "), "one\ntwo");
        assert_eq!(normalize("  lead\r\n"), "  lead");
    }

    #[test]
    fn clear_keys_walk_every_row() {
        assert_eq!(clear_keys(1), ["ctrl+e", "ctrl+u", "backspace"]);
        assert_eq!(
            clear_keys(3),
            [
                "down",
                "down",
                "ctrl+e",
                "ctrl+u",
                "backspace",
                "ctrl+e",
                "ctrl+u",
                "backspace",
                "ctrl+e",
                "ctrl+u",
                "backspace"
            ]
        );
        assert!(
            clear_keys(40)
                .iter()
                .all(|k| ["down", "ctrl+e", "ctrl+u", "backspace"].contains(k))
        );
    }
}
