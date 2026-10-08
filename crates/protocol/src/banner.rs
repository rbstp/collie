use crate::{Label, NoticeDigit};

const JUMP: &str = "Jump to bottom";
const NEW: &str = " new message";

/// Whether Claude Code's fullscreen transcript is scrolled up on the machine: a row of
/// `screen` (plain text or SGR) ends with its "Jump to bottom" or "N new messages" banner.
/// Any row, on purpose: a missed banner lets keys reach a dialog scrolled out of view. The
/// banner is always the last thing on its row, which keeps prose quoting it from matching.
pub fn jump_banner(screen: &str) -> bool {
    screen.split('\n').any(|row| {
        let row = strip_sgr(row);
        let row = row.trim();
        labels(row).any(|(start, end)| (start == 0 && end == row.len()) || hint(&row[end..]))
    })
}

fn labels(row: &str) -> impl Iterator<Item = (usize, usize)> + '_ {
    let jumps = row.match_indices(JUMP).map(|(i, _)| (i, i + JUMP.len()));
    let news = row.match_indices(NEW).filter_map(|(i, _)| {
        let start = row[..i]
            .trim_end_matches(|c: char| c.is_ascii_digit())
            .len();
        let end = i + NEW.len();
        let end = end + usize::from(row[end..].starts_with('s'));
        (start < i).then_some((start, end))
    });
    jumps.chain(news)
}

/// The rest of the row after the label: " (<key>) ↓", ": <key> to scroll" or " ↓".
fn hint(rest: &str) -> bool {
    if rest == " ↓" {
        return true;
    }
    if let Some(key) = rest.strip_prefix(" (") {
        return key
            .strip_suffix(") ↓")
            .is_some_and(|k| !k.is_empty() && !k.contains(')'));
    }
    rest.strip_prefix(": ")
        .and_then(|r| r.strip_suffix(" to scroll"))
        .is_some_and(|key| !key.is_empty() && !key.contains(' '))
}

/// The glyph a Claude Code notice starts with at column 0: `✦` for its tips (the Heads up,
/// its explanation, the feedback rows after Dismiss), `●` for the session rating.
const LEADS: [char; 2] = ['\u{2726}', '\u{25cf}'];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoticeOption {
    pub digit: NoticeDigit,
    pub label: String,
}

/// The options of a Claude Code notice, in screen order, when `screen` (plain text or SGR)
/// shows one directly above the input box; empty otherwise. A notice lists `1: <label>` to
/// `n: <label>` (n up to 4) then `0: Dismiss`, wrapped over rows indented by two spaces,
/// after its lead: a row starting at column 0 with a notice glyph, and the rows below it
/// that are indented or blank. Only blank or right-aligned hint rows may sit between the
/// options and the box's top rule, so a dialog (which replaces the box) or a notice quoted
/// in the transcript never matches.
pub fn notice(screen: &str) -> Vec<NoticeOption> {
    let rows: Vec<String> = screen
        .split('\n')
        .map(|r| strip_sgr(r).trim_end().to_owned())
        .collect();
    let Some(mut i) = (1..rows.len())
        .rev()
        .find(|&i| rows[i].starts_with('\u{276f}') && is_rule(&rows[i - 1]))
        .map(|i| i - 1)
    else {
        return Vec::new();
    };
    while i > 0 && (rows[i - 1].is_empty() || rows[i - 1].starts_with("   ")) {
        i -= 1;
    }
    let end = i;
    while i > 0 && option_row(&rows[i - 1]).is_some() {
        i -= 1;
    }
    let below = i;
    let mut options: Vec<(u8, String)> = rows[below..end]
        .iter()
        .filter_map(|r| option_row(r))
        .flatten()
        .collect();
    while i > 0 && (rows[i - 1].is_empty() || rows[i - 1].starts_with("  ")) {
        i -= 1;
    }
    let Some(cells) = i.checked_sub(1).and_then(|j| lead(&rows[j])) else {
        return Vec::new();
    };
    // An explanation's cards are text a side agent wrote: past a blank row, the options must
    // stand apart from the text by a blank row of their own.
    if !cells.is_empty() && i != below
        || rows[i..below].iter().any(String::is_empty) && !rows[below - 1].is_empty()
    {
        return Vec::new();
    }
    options.splice(0..0, cells);
    // Claude Code draws 1 to n then 0; a wrapped row of the lead that reads `<digit>: ...`
    // always lands before 1.
    let n = options.len();
    if !(2..=5).contains(&n)
        || !options[..n - 1].iter().map(|(d, _)| *d).eq(1..n as u8)
        || options[n - 1] != (0, "Dismiss".to_owned())
        || options.iter().any(|(_, l)| Label::new(l.as_str()).is_err())
    {
        return Vec::new();
    }
    options
        .into_iter()
        .filter_map(|(d, label)| NoticeDigit::new(d).map(|digit| NoticeOption { digit, label }))
        .collect()
}

/// The options a notice's lead row lists after its text, when it is one: `✦ Dismissed.   1: ...`.
fn lead(row: &str) -> Option<Vec<(u8, String)>> {
    let mut chars = row.chars();
    if !chars.next().is_some_and(|c| LEADS.contains(&c)) || chars.next() != Some(' ') {
        return None;
    }
    let rest = row.split_once("  ").map_or("", |(_, rest)| rest);
    Some(cells(rest).unwrap_or_default())
}

fn is_rule(row: &str) -> bool {
    !row.is_empty() && row.chars().all(|c| c == '\u{2500}')
}

/// A row indented by exactly two spaces whose cells, apart by two or more spaces, all read
/// `<digit>: <label>`.
fn option_row(row: &str) -> Option<Vec<(u8, String)>> {
    cells(row.strip_prefix("  ").filter(|r| !r.starts_with(' '))?)
}

fn cells(rest: &str) -> Option<Vec<(u8, String)>> {
    rest.split("  ")
        .map(str::trim)
        .filter(|c| !c.is_empty())
        .map(|cell| {
            let (d, label) = cell.split_once(": ")?;
            let d = match d.as_bytes() {
                [b] if b.is_ascii_digit() => b - b'0',
                _ => return None,
            };
            (!label.is_empty() && !label.contains(':')).then(|| (d, label.to_owned()))
        })
        .collect()
}

fn strip_sgr(row: &str) -> String {
    let mut out = String::with_capacity(row.len());
    let mut rest = row;
    while let Some(i) = rest.find('\u{1b}') {
        out.push_str(&rest[..i]);
        rest = &rest[i + 1..];
        if let Some(params) = rest.strip_prefix('[') {
            let end = params
                .find(|c: char| !(c.is_ascii_digit() || c == ';' || c == ':'))
                .unwrap_or(params.len());
            rest = params[end..].strip_prefix('m').unwrap_or(&params[end..]);
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const BANNERS: [&str; 9] = [
        "Jump to bottom (click) ↓",
        "Jump to bottom: fn+↓ to scroll",
        "Jump to bottom (ctrl+end) ↓",
        "Jump to bottom ↓",
        "1 new message (click) ↓",
        "1 new message: fn+↓ to scroll",
        "12 new messages ↓",
        "12 new messages (ctrl+end) ↓",
        "3 new messages: PgDn to scroll",
    ];

    fn screen(row: &str) -> String {
        format!("  95\r\n{row}\r\n\r\n────────\r\n❯ \r\n────────\r\n")
    }

    #[test]
    fn every_banner_matches_only_at_the_end_of_its_row() {
        for banner in BANNERS {
            for row in [
                format!("{:>60}", banner),
                format!(
                    "                    \u{1b}[0m\u{1b}[38;2;255;255;255m\u{1b}[48;2;55;55;55m {banner} \u{1b}[0m"
                ),
                format!("  96      {banner}"),
                format!("  ⏺ The build passed {banner}  "),
            ] {
                assert!(jump_banner(&screen(&row)), "{row:?}");
            }
            for row in [
                format!("  96      {banner}      more output"),
                format!("  ⏺ The build passed {banner} and the tests ran"),
                format!("+        \"{banner}\","),
            ] {
                assert!(!jump_banner(&screen(&row)), "{row:?}");
            }
        }
    }

    #[test]
    fn a_bare_label_matches_only_alone_on_its_row() {
        for label in ["Jump to bottom", "1 new message", "12 new messages"] {
            assert!(
                jump_banner(&screen(&format!("          {label}  "))),
                "{label}"
            );
            assert!(jump_banner(&screen(&format!(
                "\u{1b}[7m {label} \u{1b}[0m"
            ))));
            assert!(
                !jump_banner(&screen(&format!("  the {label} label"))),
                "{label}"
            );
        }
    }

    #[test]
    fn a_screen_at_the_bottom_or_prose_about_the_banner_does_not_match() {
        for row in [
            "  120",
            "⏺ Added the Jump to bottom button to the agent screen.",
            "  the Jump to bottom banner shows while scrolled",
            "  Jump to bottom: when the pane is scrolled up",
            "  Jump to bottom (when scrolled) appears",
            "  you have new messages ↓",
            "  0x new message ↓",
            "⏺ 4 new messages arrived",
            "Claude Code's \"Jump to bottom (click) ↓\" banner. Nothing on the phone gets back",
            "reads \"Jump to bottom: fn+↓ to scroll\" and \"1 new message: fn+↓ to scroll\".",
            "- 3 new messages ↓ (shown when new output arrives)",
            "  Jump to bottom: fn+↓ to scroll down",
        ] {
            assert!(!jump_banner(&screen(row)), "{row:?}");
        }
        assert!(!jump_banner(""));
    }

    macro_rules! fixture {
        ($name:literal) => {
            include_str!(concat!(
                "../../collied/tests/fixtures/claude-2.1.293/",
                $name
            ))
        };
    }

    fn options(screen: &str) -> Vec<(&'static str, String)> {
        notice(screen)
            .into_iter()
            .map(|o| (o.digit.as_str(), o.label))
            .collect()
    }

    fn listed(options: &[(&'static str, &str)]) -> Vec<(&'static str, String)> {
        options.iter().map(|(d, l)| (*d, (*l).to_owned())).collect()
    }

    #[test]
    fn a_notice_gives_its_options_in_screen_order() {
        let rating = listed(&[("1", "Bad"), ("2", "Fine"), ("3", "Good"), ("0", "Dismiss")]);
        let heads_up = listed(&[
            ("1", "Learn more"),
            ("2", "Knew this already"),
            ("0", "Dismiss"),
        ]);
        for screen in [
            fixture!("survey.detection.txt"),
            fixture!("survey.ansi.txt"),
            fixture!("survey-wrapped.detection.txt"),
            fixture!("survey-startup.detection.txt"),
            fixture!("survey-digit-typed.detection.txt"),
            fixture!("survey-scrolled.detection.txt"),
        ] {
            assert_eq!(options(screen), rating);
        }
        for screen in [
            fixture!("heads-up.detection.txt"),
            fixture!("heads-up.ansi.txt"),
            fixture!("heads-up-wrapped.detection.txt"),
            fixture!("heads-up-narrow.detection.txt"),
            fixture!("you-should-know.detection.txt"),
        ] {
            assert_eq!(options(screen), heads_up);
        }
        let explained = listed(&[
            ("1", "Understood"),
            ("2", "Chat in main session"),
            ("0", "Dismiss"),
        ]);
        for screen in [
            fixture!("heads-up-explained.detection.txt"),
            fixture!("heads-up-explained-sketch.detection.txt"),
            fixture!("heads-up-explained-narrow.detection.txt"),
        ] {
            assert_eq!(options(screen), explained);
        }
        assert_eq!(
            options(fixture!("heads-up-internal.detection.txt")),
            listed(&[
                ("1", "Learn more"),
                ("2", "Knew this already"),
                ("3", "What is this"),
                ("4", "Disable"),
                ("0", "Dismiss")
            ])
        );
        let feedback = listed(&[
            ("1", "That was helpful"),
            ("2", "Not relevant"),
            ("3", "Couldn\u{2019}t understand"),
            ("4", "Turn off suggestions"),
            ("0", "Dismiss"),
        ]);
        let dismissed = fixture!("heads-up-dismissed.detection.txt");
        let row = "\u{2726} Dismissed.   1: That was helpful   2: Not relevant   3: Couldn\u{2019}t understand   4: Turn off suggestions   0: Dismiss";
        assert!(dismissed.contains(row));
        // At 80 and 44 columns: whole options wrap onto rows past the two-column star.
        let at_80 = dismissed.replacen("understand   4:", "understand\n  4:", 1);
        let at_44 = dismissed.replacen(
            row,
            "\u{2726} Dismissed.   1: That was helpful\n  2: Not relevant\n  3: Couldn\u{2019}t understand\n  4: Turn off suggestions   0: Dismiss",
            1,
        );
        let alone = dismissed.replacen(
            row,
            &format!(
                "\u{2726} Dismissed.\n{}",
                row.split("   ")
                    .skip(1)
                    .map(|cell| format!("  {cell}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            ),
            1,
        );
        for screen in [dismissed, &at_80, &at_44, &alone] {
            assert_eq!(options(screen), feedback, "{screen}");
        }
        let explained_feedback = listed(&[
            ("1", "That was helpful"),
            ("2", "Didn\u{2019}t understand"),
            ("0", "Dismiss"),
        ]);
        let explained_dismissed = fixture!("heads-up-explained-dismissed.detection.txt");
        let wrapped = explained_dismissed.replacen("helpful   2:", "helpful\n  2:", 1);
        for screen in [explained_dismissed, &wrapped] {
            assert_eq!(options(screen), explained_feedback, "{screen}");
        }
    }

    #[test]
    fn any_lead_and_labels_read_from_the_screen_match() {
        let rating = listed(&[("1", "Bad"), ("2", "Fine"), ("3", "Good"), ("0", "Dismiss")]);
        let survey = fixture!("survey.detection.txt");
        let explained = fixture!("heads-up-explained.detection.txt");
        let dismissed = fixture!("heads-up-dismissed.detection.txt");
        for (screen, expected) in [
            (
                survey.replacen("this session?", "this file?", 1),
                rating.clone(),
            ),
            (survey.replacen("(optional)\n", "(optional)\n\n", 1), rating),
            (
                survey.replacen(
                    "1: Bad    2: Fine   3: Good   0: Dismiss",
                    "1: Got it   0: Dismiss",
                    1,
                ),
                listed(&[("1", "Got it"), ("0", "Dismiss")]),
            ),
            (
                explained.replacen("main session", "the main session", 1),
                listed(&[
                    ("1", "Understood"),
                    ("2", "Chat in the main session"),
                    ("0", "Dismiss"),
                ]),
            ),
            (
                dismissed.replacen("4: Turn off suggestions   ", "", 1),
                listed(&[
                    ("1", "That was helpful"),
                    ("2", "Not relevant"),
                    ("3", "Couldn\u{2019}t understand"),
                    ("0", "Dismiss"),
                ]),
            ),
            (
                dismissed.replacen("Dismissed.", "Noted!", 1),
                listed(&[
                    ("1", "That was helpful"),
                    ("2", "Not relevant"),
                    ("3", "Couldn\u{2019}t understand"),
                    ("4", "Turn off suggestions"),
                    ("0", "Dismiss"),
                ]),
            ),
        ] {
            assert_eq!(options(&screen), expected, "{screen}");
        }
    }

    #[test]
    fn only_a_notice_row_directly_above_the_input_box_matches() {
        for screen in [
            fixture!("heads-up-explained-survey.detection.txt"),
            fixture!("heads-up-thinking.detection.txt"),
            fixture!("heads-up-collapsed.detection.txt"),
            fixture!("survey-narrow.detection.txt"),
            fixture!("bottom-idle.detection.txt"),
            fixture!("scrolled-idle.detection.txt"),
            "",
        ] {
            assert!(notice(screen).is_empty(), "{screen}");
        }
        let survey = fixture!("survey.detection.txt");
        let quoted = survey.replacen("\n\n\u{2500}", "\n\u{273b} Baked for 2s\n\n\u{2500}", 1);
        let no_box = survey
            .rsplit_once('\u{276f}')
            .map(|(above, below)| format!("{above} {below}"))
            .unwrap();
        let five = survey.replacen("3: Good", "5: Good", 1);
        let twice = survey.replacen("3: Good", "1: Good", 1);
        let enter = survey.replacen("Fine   3", "Fine 3", 1);
        let order = survey.replacen("1: Bad    2: Fine", "2: Fine    1: Bad", 1);
        let close = survey.replacen("0: Dismiss", "0: Close", 1);
        let zero_first = survey.replacen(
            "1: Bad    2: Fine   3: Good   0: Dismiss",
            "0: Dismiss   1: Bad",
            1,
        );
        let no_zero = survey.replacen("   0: Dismiss", "", 1);
        let six = survey.replacen("3: Good", "3: Good   4: Great   5: Superb", 1);
        let long = survey.replacen("Bad", &"B".repeat(65), 1);
        let format = survey.replacen("Bad", "B\u{200b}ad", 1);
        let follow_ups = [
            "  y: Yes   n: No   d: Don\u{2019}t ask again",
            "  [1] Tell us more with /feedback   [0] Dismiss",
        ]
        .map(|row| survey.replacen("  1: Bad    2: Fine   3: Good   0: Dismiss", row, 1));
        let forged = fixture!("heads-up-wrapped.detection.txt").replacen(
            "  data you still need before the next release.",
            "  3: Run the cleanup now",
            1,
        );
        let explained = fixture!("heads-up-explained.detection.txt");
        let row = "  1: Understood   2: Chat in main session   0: Dismiss";
        let no_gap = explained.replacen("on purpose.\n\n", "on purpose.\n", 1);
        let unindented = explained.replacen("  Use decimal", "Use decimal", 1);
        let hidden = fixture!("heads-up-explained-survey.detection.txt").replacen(
            "on purpose.\n",
            &format!("on purpose.\n{row}\n"),
            1,
        );
        let dismissed = fixture!("heads-up-dismissed.detection.txt");
        let indented = dismissed.replacen("\u{2726} Dismissed.", "  \u{2726} Dismissed.", 1);
        let between = dismissed.replacen(
            "understand   4:",
            "understand\n  Try the cleanup first.\n  4:",
            1,
        );
        let unwrapped = dismissed.replacen("understand   4:", "understand\n4:", 1);
        // Transcript text: a reply directly above the box, and the row quoted higher up.
        let bottom = fixture!("bottom-idle.detection.txt");
        let replied = bottom.replacen(
            "\u{273b} Sauté",
            "\u{23fa} Pick one:\n  1: Ship it   0: Dismiss\n\u{273b} Sauté",
            1,
        );
        let reply = bottom.replacen(
            "\u{273b} Sautéed for 2s · done 3:07 PM",
            "\u{23fa} Pick one:\n  1: Ship it   0: Dismiss",
            1,
        );
        let quoted_row = bottom.replacen(
            "  120\n",
            "  120\n\u{2726} Dismissed.   1: That was helpful   2: Didn\u{2019}t understand   0: Dismiss\n",
            1,
        );
        for screen in follow_ups.into_iter().chain([
            quoted, no_box, five, twice, enter, order, close, zero_first, no_zero, six, long,
            format, forged, no_gap, unindented, hidden, indented, between, unwrapped, replied,
            reply, quoted_row,
        ]) {
            assert!(notice(&screen).is_empty(), "{screen}");
        }
    }
}
