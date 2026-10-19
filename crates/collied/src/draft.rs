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
    let mut wrap = Wrap::default();
    let mut after_hidden = false;
    for (i, line) in lines.iter().enumerate() {
        if i > 0 && !std::mem::take(&mut after_hidden) {
            let prev = &lines[i - 1];
            after_hidden = line.is_empty() && wrap.blank(prev, &lines[i + 1..], width);
            text.push_str(if after_hidden {
                " "
            } else {
                wrap.join(prev, line, &lines[i + 1..], width)
            });
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

/// A soft wrap is a row the next word did not fit on, or a full row that split a word
/// longer than a row; a row starting with a space follows a typed newline. A typed newline
/// right where the next word would not have fit reads as a space.
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

/// Claude Code 2.1 wraps the draft at the terminal width (the rule's) minus 4, and its
/// prose, with wrap-ansi `{hard: true, trim: false}` and hides the spaces around each soft
/// wrap. After a full row, the space before the next word still takes the next row's first
/// column, unseen, so that row holds one column less. This is what a row leaves for the one
/// below it.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Wrap {
    hidden: bool,
    /// Where the word this row starts with began, and its width on the rows above.
    first: (Start, usize),
}

/// wrap-ansi starts a word longer than a row where it is, after its space, unless that
/// takes more rows than starting it on a new row; a word that does not fit moves to a new row.
#[derive(Debug, Clone, Copy)]
enum Start {
    At(usize),
    /// Moved off a row with this many columns used, its space included.
    Moved(usize),
}

impl Default for Start {
    fn default() -> Self {
        Start::At(0)
    }
}

impl Wrap {
    /// Whether `prev`, the row this describes, is full and the blank row after it holds only
    /// the hidden space: the word the first of `below` starts with would not fit after it.
    pub(crate) fn blank<S: AsRef<str>>(
        &mut self,
        prev: &str,
        below: impl IntoIterator<Item = S>,
        width: usize,
    ) -> bool {
        let mut below = below.into_iter();
        let Some(next) = below.next() else {
            return false;
        };
        let next = next.as_ref();
        let (used, start, len) = self.last_word(prev);
        let blank = width > 0
            && !prev.is_empty()
            && used >= width
            && fits(start, len, width)
            && !next.is_empty()
            && !next.starts_with(char::is_whitespace)
            && ends(next, below, width)
                .last()
                .is_some_and(|l| pushed(1, l, width));
        if blank {
            *self = Wrap {
                hidden: false,
                first: (Start::Moved(1), 0),
            };
        }
        blank
    }

    /// What joins `next` to `prev`, the row this describes: `"\n"` for a typed newline,
    /// `" "` for a soft wrap, `""` for a split word. `below` are the rows after `next`, onto
    /// which the word `next` starts with may split. Where the rows could come from more than
    /// one text: a full row of one word, not after a hidden space, splits onto the next as
    /// far as the rows allow; a word reads as long as that, then after a soft wrap rather
    /// than a typed newline; after any other full row, a soft wrap reads before a split word.
    pub(crate) fn join<S: AsRef<str>>(
        &mut self,
        prev: &str,
        next: &str,
        below: impl IntoIterator<Item = S>,
        width: usize,
    ) -> &'static str {
        let row = std::mem::take(self);
        if width == 0 || prev.is_empty() || next.is_empty() || next.starts_with(char::is_whitespace)
        {
            return "\n";
        }
        let (used, start, len) = row.last_word(prev);
        let one_word = !prev.contains(' ');
        let ends = ends(next, below, width);
        let sep = if used >= width {
            let split = splits(start, len, ends, width);
            if split && one_word && !row.hidden {
                ""
            } else if fits(start, len, width) && next.width() < width {
                " "
            } else if split {
                ""
            } else {
                "\n"
            }
        } else if ends.last().is_some_and(|l| pushed(used + 1, l, width)) {
            " "
        } else {
            "\n"
        };
        *self = match sep {
            "" => row.split(prev),
            " " if used >= width => Wrap {
                hidden: true,
                first: (Start::At(1), 0),
            },
            " " => Wrap {
                hidden: false,
                first: (Start::Moved(used + 1), 0),
            },
            _ => Wrap::default(),
        };
        sep
    }

    /// Whether the word `prev`, the row this describes, ends with may split onto `next` and
    /// the rows `below` it.
    pub(crate) fn continues<S: AsRef<str>>(
        self,
        prev: &str,
        next: &str,
        below: impl IntoIterator<Item = S>,
        width: usize,
    ) -> bool {
        let (_, start, len) = self.last_word(prev);
        splits(start, len, ends(next, below, width), width)
    }

    /// The row after `prev`, the row this describes, where a split word joins them.
    pub(crate) fn split(self, prev: &str) -> Wrap {
        let (_, start, len) = self.last_word(prev);
        Wrap {
            hidden: false,
            first: (start, len),
        }
    }

    /// The columns `prev` uses, and where the word it ends with began and how wide it is so
    /// far.
    fn last_word(self, prev: &str) -> (usize, Start, usize) {
        let used = usize::from(self.hidden) + prev.width();
        let last = prev.rsplit(' ').next().unwrap_or_default().width();
        if prev.contains(' ') {
            (used, Start::At(used - last), last)
        } else {
            (used, self.first.0, self.first.1 + last)
        }
    }
}

/// Whether wrap-ansi moves a word `len` columns wide to a new row when `used` columns of the
/// row hold the text before it and its space.
fn pushed(used: usize, len: usize, width: usize) -> bool {
    if len > width {
        (len - 1) / width < 1 + (len + used - width - 1) / width
    } else {
        used + len > width
    }
}

fn splits(start: Start, len: usize, mut ends: impl Iterator<Item = usize>, width: usize) -> bool {
    ends.any(|l| len + l > width && fits(start, len + l, width))
}

fn fits(start: Start, len: usize, width: usize) -> bool {
    match start {
        Start::At(at) => !pushed(at, len, width),
        Start::Moved(from) => pushed(from, len, width),
    }
}

/// The widths the word `row` starts with may have: it ends on `row` unless that is a full
/// row of it, then on each full row of it below.
fn ends<S: AsRef<str>>(
    row: &str,
    below: impl IntoIterator<Item = S>,
    width: usize,
) -> impl Iterator<Item = usize> {
    let mut below = below.into_iter();
    let mut row = Some(row.to_owned());
    let mut len = 0;
    std::iter::from_fn(move || {
        let text = row.take()?;
        let first = text.split(' ').next().unwrap_or_default();
        if first.is_empty() {
            return None;
        }
        len += first.width();
        if first.len() == text.len() && text.width() >= width {
            row = below.next().map(|next| next.as_ref().to_owned());
        }
        Some(len)
    })
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

/// A model of Claude Code 2.1's wrap to draw screens from: wrap-ansi `{hard: true, trim:
/// false}` with the spaces around each soft wrap hidden.
#[cfg(test)]
pub mod model {
    use proptest::prelude::*;
    use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

    /// Lines of single-spaced words, some longer than a row or of wide characters.
    pub fn text() -> impl Strategy<Value = String> {
        let line = prop::collection::vec("[a-z]{1,12}|[中文]{1,4}", 0..8).prop_map(|w| w.join(" "));
        prop::collection::vec(line, 1..4).prop_map(|l| l.join("\n"))
    }

    /// The rows of `text` wrapped at `width`, each after what joins it to the row above:
    /// `"\n"` for a typed newline, `" "` for a soft wrap, `""` for a split word.
    pub fn rows(text: &str, width: usize) -> Vec<(&'static str, String)> {
        wrap(text, width)
            .into_iter()
            .map(|(sep, row)| match row.strip_prefix(' ') {
                Some(rest) if sep == " " => (sep, rest.to_owned()),
                _ => (sep, row),
            })
            .collect()
    }

    /// wrap-ansi's rows, where a space after a full row still starts the next one.
    fn wrap(text: &str, width: usize) -> Vec<(&'static str, String)> {
        let mut rows: Vec<(&str, String)> = Vec::new();
        for line in text.split('\n') {
            rows.push(("\n", String::new()));
            let mut used = 0;
            for (i, word) in line.split(' ').enumerate() {
                if i > 0 {
                    if used >= width {
                        rows.push((" ", String::new()));
                        used = 0;
                    }
                    rows.last_mut().unwrap().1.push(' ');
                    used += 1;
                }
                let len = word.width();
                if len > width {
                    if (len - 1) / width < 1 + (len - (width - used) - 1) / width {
                        rows.push((" ", String::new()));
                        used = 0;
                    }
                    for (j, c) in word.chars().enumerate() {
                        let w = c.width().unwrap_or(0);
                        if used > 0 && used + w > width {
                            rows.push((if j == 0 { " " } else { "" }, String::new()));
                            used = 0;
                        }
                        rows.last_mut().unwrap().1.push(c);
                        used += w;
                    }
                } else if used > 0 && used + len > width {
                    rows.push((" ", word.to_owned()));
                    used = len;
                } else {
                    rows.last_mut().unwrap().1.push_str(word);
                    used += len;
                }
            }
        }
        rows
    }

    /// Whether `read`, which a reader took from the rows of `text`, draws them too and reads
    /// first where the two part, in the order [`super::Wrap::join`] gives, with a run of wide
    /// characters one column short of the edge splitting first, as `reflow` reads it.
    pub fn reads_first(text: &str, read: &str, width: usize) -> bool {
        let read = format!("{}{}", read.trim_end(), &text[text.trim_end().len()..]);
        let (typed, other) = (rows(text, width), rows(&read, width));
        let shown = |rows: &[(&str, String)]| -> Vec<String> {
            rows.iter().map(|(_, r)| r.trim_end().to_owned()).collect()
        };
        let Some(k) = (1..typed.len()).find(|&k| other.get(k).is_some_and(|o| o.0 != typed[k].0))
        else {
            return false;
        };
        if shown(&typed) != shown(&other) {
            return false;
        }
        let one_word = !wrap(text, width)[k - 1].1.trim_end().contains(' ');
        let above = typed[k - 1].1.trim_end();
        let wide = |c: Option<char>| c.and_then(UnicodeWidthChar::width) == Some(2);
        let cut = above.width() + 1 == width
            && wide(above.chars().next_back())
            && wide(typed[k].1.chars().next());
        let rank = |sep: &str| match sep {
            "\n" => 0,
            "" if cut => 3,
            "" if one_word => 2,
            " " if !one_word => 2,
            _ => 1,
        };
        let word = |rows: &[(&str, String)]| {
            let mut j = k + usize::from(rows[k].1.trim_end().is_empty());
            let mut len = 0;
            while let Some((_, row)) = rows.get(j) {
                let row = row.trim_end();
                let first = row.split(' ').next().unwrap_or_default();
                len += first.width();
                j += 1;
                if first.len() < row.len() || rows.get(j).is_none_or(|r| !r.0.is_empty()) {
                    break;
                }
            }
            len
        };
        let (a, b) = (typed[k].0, other[k].0);
        if a.is_empty() || b.is_empty() {
            rank(b) > rank(a)
        } else {
            (word(&other), rank(b)) > (word(&typed), rank(a))
        }
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

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
    fn rows_after_a_full_row_hold_a_column_less() {
        // Claude Code 2.1.302 draws these at 44 columns.
        let (a, b, c20, c22, c44, d23) = (
            "a".repeat(20),
            "b".repeat(23),
            "c".repeat(20),
            "c".repeat(22),
            "c".repeat(44),
            "d".repeat(23),
        );
        let full = format!("{a} {b}");
        let rule = "─".repeat(48);
        let read = |rows: &[&str]| parse(&format!("{rule}\n❯ {}\n{rule}\n", rows.join("\n  ")));
        assert_eq!(read(&[&full, &c22]), draft(&format!("{full} {c22}"), 2));
        assert_eq!(
            read(&["aaaa", &"b".repeat(44), "bbbbbb"]),
            draft(&format!("aaaa\n{}", "b".repeat(50)), 3),
            "a typed newline before a word that would have started on the row"
        );
        assert_eq!(read(&[&full, "", &c44]), draft(&format!("{full} {c44}"), 3));
        assert_eq!(
            read(&[&full, &c20, &d23]),
            draft(&format!("{full} {c20} {d23}"), 3)
        );
        assert_eq!(
            read(&[&full, &format!("{c20} {d23}")]),
            draft(&format!("{full}\n{c20} {d23}"), 2),
            "a soft-wrapped row after a full row cannot fill the box"
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

    proptest! {
        #[test]
        fn reads_back_what_was_typed(
            (text, width) in (model::text(), 4..24usize).prop_filter("unreadable", |(t, w)| {
                // A run of wide characters split one column short of the edge reads as two
                // words.
                !model::rows(t, *w)
                    .windows(2)
                    .any(|r| r[1].0.is_empty() && r[0].1.width() < *w)
            }),
        ) {
            let rows: Vec<String> = model::rows(&text, width).into_iter().map(|(_, r)| r).collect();
            let rule = "─".repeat(width + 4);
            let screen = format!("{rule}\n❯ {}\n{rule}\n", rows.join("\n  "));
            let Some(InputBox::Draft(read)) = parse(&screen) else {
                return Err(TestCaseError::fail("no draft"));
            };
            prop_assert_eq!(read.lines, rows.len());
            let typed = normalize(&text);
            prop_assert!(
                read.text == typed || model::reads_first(&text, &read.text, width),
                "{:?} read as {:?}",
                typed,
                read.text
            );
        }
    }

    #[test]
    fn model_wraps_like_wrap_ansi() {
        // wrap-ansi draws `["aaaa aaa", " b cc ", "ddd"]`.
        assert_eq!(
            model::rows("aaaa aaa b cc ddd", 8),
            [
                ("\n", "aaaa aaa".to_owned()),
                (" ", "b cc ".to_owned()),
                (" ", "ddd".to_owned())
            ]
        );
        // Claude Code 2.1.302 draws these at 44 columns: the space after a full row takes a
        // column of the next row, alone on it before a word that fills a row.
        let full = format!("{} {}", "a".repeat(20), "b".repeat(23));
        let shown = |text: &str| -> Vec<String> {
            model::rows(text, 44).into_iter().map(|(_, r)| r).collect()
        };
        assert_eq!(
            shown(&format!("{full} {} {}", "c".repeat(20), "d".repeat(23))),
            [full.clone(), "c".repeat(20) + " ", "d".repeat(23)]
        );
        assert_eq!(
            shown(&format!("{full} {}", "c".repeat(44))),
            [full.clone(), String::new(), "c".repeat(44)]
        );
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
