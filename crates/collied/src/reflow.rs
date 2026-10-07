use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::draft::separator;

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
/// or `※` row counts; a row that only might continue the one above never does.
pub fn soft_wraps(ansi: &str, cols: usize) -> (Vec<u32>, Vec<u32>) {
    let (mut wraps, mut splits) = (Vec::new(), Vec::new());
    let mut prose = false;
    let mut prev = Prev::Start;
    let (mut plain, mut tail) = (String::new(), String::new());
    for (i, row) in ansi.split('\n').enumerate() {
        plain_text(row, &mut plain);
        let trimmed = plain.trim_end();
        let s = trimmed.trim_start_matches(' ');
        let ind = trimmed.len() - s.len();
        if s.is_empty() {
            prev = Prev::Start;
            continue;
        }
        if ind == 0 {
            let body = s.strip_prefix(['⏺', '※']);
            prose = body.is_some();
            prev = match body.map(|b| b.trim_start_matches([' ', '\u{a0}'])) {
                Some(b) if !tool_header(b) => row_above(&mut tail, b, 2),
                _ => Prev::Held,
            };
            continue;
        }
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
                    match separator(&tail, s, width) {
                        " " if wide_cut(&tail, s, width) => splits.push(at),
                        " " => wraps.push(at),
                        "" => splits.push(at),
                        _ => {}
                    }
                }
                row_above(&mut tail, s, ind)
            }
            Prev::Start => row_above(&mut tail, s, ind),
            Prev::Row(..) | Prev::Held => Prev::Held,
        };
    }
    (wraps, splits)
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
    use super::*;

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
    fn wide_characters_cut_short_of_the_edge_split() {
        let row = format!("⏺ {}", "中".repeat(93));
        let rows = [row.as_str(), "  文字。"];
        assert_eq!(joins(&rows, 189), (vec![], vec![1]));
        assert_eq!(joins(&rows, 190), (vec![1], vec![]));
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
