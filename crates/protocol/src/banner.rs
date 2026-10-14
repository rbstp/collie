use unicode_width::UnicodeWidthStr;

use crate::{Label, NoticeDigit};

const JUMP: &str = "Jump to bottom";
const NEW: &str = " new message";

/// Whether Claude Code's fullscreen transcript is scrolled up on the machine: `screen` (SGR)
/// shows its "Jump to bottom" or "N new messages" banner. Any row, on purpose: a missed banner
/// lets keys reach a dialog scrolled out of view. Claude Code draws the banner over whatever
/// row is there, centered, as a run on a background color padded by a space on each side. Over
/// a prompt's text, drawn in the same style, no escape separates them, so the banner is then
/// found inside the run, by its padding and its column. Replies quoting it have no background;
/// prompt text matches when a run of it is exactly a banner with a space on each side, or when
/// it holds one, so padded, centered on a row as wide as the screen. Columns are display
/// widths, as the terminal lays them out.
pub fn jump_banner(screen: &str) -> bool {
    screen.split('\n').any(|row| {
        let row = row.strip_suffix('\r').unwrap_or(row);
        let width = runs(row).map(|(_, run)| run.width()).sum();
        let mut column = 0;
        runs(row).any(|(background, run)| {
            let at = column;
            column += run.width();
            background && (banner(run) || centered(run, at, width))
        })
    })
}

fn banner(run: &str) -> bool {
    let Some(text) = run.strip_prefix(' ').and_then(|r| r.strip_suffix(' ')) else {
        return false;
    };
    let text = text.trim();
    labels(text).any(|(start, end)| start == 0 && (end == text.len() || hint(&text[end..])))
}

/// A label and its hint inside `run`, which starts at column `at`, padded by a space on each
/// side and centered on a row `width` columns wide.
fn centered(run: &str, at: usize, width: usize) -> bool {
    labels(run).any(|(start, end)| {
        let rest = &run[end..];
        let Some(stop) = [" \u{2193}", " to scroll"]
            .iter()
            .filter_map(|tail| rest.find(tail).map(|i| end + i + tail.len()))
            .find(|&stop| hint(&run[end..stop]))
        else {
            return false;
        };
        let column = at + run[..start].width();
        let len = run[start..stop].width();
        run[..start].ends_with(' ')
            && run[stop..].starts_with(' ')
            && (2 * column + len).abs_diff(width) <= 1
    })
}

/// The row's text between escape sequences, each with whether an SGR background is set.
fn runs(row: &str) -> impl Iterator<Item = (bool, &str)> {
    let mut background = false;
    row.split('\u{1b}').enumerate().map(move |(i, part)| {
        let Some(params) = part.strip_prefix('[').filter(|_| i > 0) else {
            return (background, part);
        };
        let end = params
            .find(|c: char| !(c.is_ascii_digit() || c == ';' || c == ':'))
            .unwrap_or(params.len());
        match params[end..].strip_prefix('m') {
            Some(text) => {
                background = sets_background(&params[..end], background);
                (background, text)
            }
            None => (background, &params[end..]),
        }
    })
}

fn sets_background(params: &str, mut background: bool) -> bool {
    let mut ps = params.split(';');
    while let Some(p) = ps.next() {
        let code = p.split(':').next().unwrap_or_default();
        match code {
            "" | "0" | "49" => background = false,
            "38" | "48" | "58" => {
                // The color's own parameters, which would otherwise read as codes.
                if !p.contains(':') {
                    match ps.next() {
                        Some("5") => _ = ps.next(),
                        Some("2") => _ = ps.nth(2),
                        _ => {}
                    }
                }
                background |= code == "48";
            }
            _ => background |= matches!(code.parse::<u8>(), Ok(40..=47 | 100..=107)),
        }
    }
    background
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
/// that are indented or blank. At most three blank rows and right-aligned hint rows may sit
/// between the options and the box's top rule, so a dialog (which replaces the box) or a
/// notice quoted in the transcript never matches.
pub fn notice(screen: &str) -> Vec<NoticeOption> {
    let rows: Vec<String> = screen
        .split('\n')
        .map(|r| strip_sgr(r).trim_end().to_owned())
        .collect();
    let Some(rule) = (1..rows.len())
        .rev()
        .find(|&i| rows[i].starts_with('\u{276f}'))
        .filter(|&i| is_rule(&rows[i - 1]))
        .map(|i| i - 1)
    else {
        return Vec::new();
    };
    // The box's one-row top margin, which a hint draws over, and the two blank rows a real
    // screen showed above it.
    let mut i = rule;
    let mut blanks = 0;
    while i > 0 && (rows[i - 1].starts_with("   ") || rows[i - 1].is_empty() && blanks < 3) {
        blanks += usize::from(rows[i - 1].is_empty());
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
    // stand apart from the text by a blank row of their own, and sit right on the box's top
    // margin or on the two blank rows above a hint, since every card ends with a blank row.
    let gap = &rows[end..rule];
    if !cells.is_empty() && i != below
        || rows[i..below].iter().any(String::is_empty)
            && (!rows[below - 1].is_empty()
                || gap.iter().rev().skip(1).any(String::is_empty)
                    && !matches!(gap, [a, b, h] if a.is_empty() && b.is_empty() && !h.is_empty()))
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

/// A row of `─`, or one carrying the session's title the way Claude Code draws it:
/// `─── <title> ─`.
fn is_rule(row: &str) -> bool {
    let Some(head) = row.strip_suffix('\u{2500}') else {
        return false;
    };
    let title = head.trim_start_matches('\u{2500}');
    row.starts_with('\u{2500}')
        && (title.is_empty()
            || title.starts_with(' ')
                && title.ends_with(' ')
                && !title.trim().is_empty()
                && !title.contains('\u{2500}'))
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

    // How Claude Code 2.1.299 draws the banner, from the captures below.
    const STYLE: &str = "\u{1b}[0m\u{1b}[38;2;255;255;255m\u{1b}[48;2;55;55;55m";
    const PROMPT_ROW: &str = "\u{1b}[48;2;55;55;55m";

    #[test]
    fn every_banner_matches_wherever_claude_code_draws_it_on_its_row() {
        for banner in BANNERS {
            let drawn = format!("{STYLE} {banner} \u{1b}[0m");
            for (i, row) in [
                format!("{:40}{drawn}", ""),
                format!("  96      {drawn}"),
                format!("  48. Overcast skies can feel gloomy, w{drawn}persist for days"),
                format!("{STYLE}  {banner}  \u{1b}[0m"),
                format!(
                    "\u{1b}[0m{PROMPT_ROW}  {STYLE}Then shows.\u{1b}[0m{PROMPT_ROW}   {drawn}{PROMPT_ROW}   \u{1b}[0m"
                ),
                format!("\u{1b}[1;38;5;4;100m {banner} \u{1b}[m"),
                format!("\u{1b}[48:2::55:55:55m {banner} \u{1b}[0m"),
            ]
            .into_iter()
            .enumerate()
            {
                assert!(jump_banner(&screen(&row)), "{i}");
            }
            for (i, row) in [
                format!("{:>60}", banner),
                format!("  96      {banner}"),
                format!("  ⏺ The build passed {banner}  "),
                format!("{STYLE}{banner}\u{1b}[0m"),
                format!(
                    "\u{1b}[0m{PROMPT_ROW}  {STYLE}{banner}\u{1b}[0m{PROMPT_ROW}      \u{1b}[0m"
                ),
                format!("\u{1b}[38;2;177;185;249m {banner} \u{1b}[0m"),
                format!("\u{1b}[38;2;48;0;0m {banner} \u{1b}[0m"),
                format!("{PROMPT_ROW}\u{1b}[49m {banner} "),
                format!("{STYLE} {banner} and more \u{1b}[0m"),
                format!("{STYLE} the {banner} \u{1b}[0m"),
            ]
            .into_iter()
            .enumerate()
            {
                assert!(!jump_banner(&screen(&row)), "{i}");
            }
        }
    }

    #[test]
    fn a_bare_label_matches_only_alone_in_its_run() {
        for label in ["Jump to bottom", "1 new message", "12 new messages"] {
            assert!(jump_banner(&screen(&format!(
                "          {STYLE} {label} \u{1b}[0m"
            ))));
            assert!(!jump_banner(&screen(&format!("          {label}  "))));
            assert!(!jump_banner(&screen(&format!(
                "  {STYLE} the {label} label \u{1b}[0m"
            ))));
        }
    }

    macro_rules! captured {
        ($name:literal) => {
            include_str!(concat!(
                "../../collied/tests/fixtures/claude-2.1.299/",
                $name
            ))
        };
    }

    #[test]
    fn a_captured_banner_matches_on_any_row_and_quoted_banners_do_not() {
        // Visible reads, scrolled up with Page Up or the mouse wheel: the banner on a blank
        // row, at the end of a short row, in the middle of a long row, over a prompt row
        // apart from its text, right after its text, and with its text on both sides, and a
        // new message banner.
        for (i, screen) in [
            captured!("jump-blank.ansi.txt"),
            captured!("jump-end-of-row.ansi.txt"),
            captured!("jump-mid-row.ansi.txt"),
            captured!("jump-over-prompt.ansi.txt"),
            captured!("jump-prompt-left.ansi.txt"),
            captured!("jump-prompt-both.ansi.txt"),
            captured!("new-message.ansi.txt"),
        ]
        .into_iter()
        .enumerate()
        {
            assert!(jump_banner(screen), "{i}");
            assert!(jump_banner(&screen.replace('\n', "\r\n")), "{i}");
            assert!(!jump_banner(&strip_sgr(screen)), "{i}");
        }
        // Columns are display widths: wide and combining characters around it keep it centered.
        for (i, swaps) in [
            vec![("read these", "\u{8aad}\u{6e56}\u{6e56}hese")],
            vec![("bered lines", "\u{6e56}\u{6e56}d lines")],
            vec![
                ("lines, th Jump", "lines, \u{1f30a} Jump"),
                ("about lakes.", "\u{1f30a}\u{1f30a}\u{1f30a}lakes."),
            ],
            vec![("read these", "re\u{301}ad the\u{301}se")],
        ]
        .into_iter()
        .enumerate()
        {
            let screen = swaps.into_iter().fold(
                captured!("jump-prompt-both.ansi.txt").to_owned(),
                |s, (from, to)| s.replacen(from, to, 1),
            );
            assert_ne!(screen, captured!("jump-prompt-both.ansi.txt"), "{i}");
            assert!(jump_banner(&screen), "{i}");
        }
        // Inside prompt text, only centered and padded.
        for (i, (from, to)) in [
            ("read these lines", "xxread these lines"),
            (
                " Jump to bottom (click) \u{2193} ",
                "-Jump to bottom (click) \u{2193} ",
            ),
            (
                " Jump to bottom (click) \u{2193} ",
                " Jump to bottom (click) \u{2193}-",
            ),
            ("(click) \u{2193}", "(click)  \u{2193}"),
        ]
        .into_iter()
        .enumerate()
        {
            let screen = captured!("jump-prompt-both.ansi.txt").replacen(from, to, 1);
            assert!(!jump_banner(&screen), "{i}");
        }
        // At the bottom, a prompt and its reply quoting each banner, on rows of their own.
        assert!(!jump_banner(captured!("quoted.ansi.txt")));
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

    const HINT: &str = "new task? /clear to save 352.8k tokens";

    #[test]
    fn a_rule_may_carry_the_session_title_only_as_claude_code_draws_it() {
        for row in ["────", "── Price rounding review (2) ─", "─ a ─"] {
            assert!(is_rule(row), "{row:?}");
        }
        for row in [
            "",
            "──── Price rounding review (2)",
            " Price rounding review (2) ─",
            "x─── Price rounding review (2) ─",
            "────Price rounding review (2)─",
            "──── one ─── two ─",
            "────   ─",
            "──── ❯ ─ x",
            "──── Price rounding review (2) ──",
            "──── 3 new messages ────",
        ] {
            assert!(!is_rule(row), "{row:?}");
        }
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
            fixture!("heads-up-titled.detection.txt"),
            fixture!("heads-up-hint.detection.txt"),
            &fixture!("heads-up-hint.detection.txt").replacen(HINT, "", 1),
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
            fixture!("heads-up-explained-hint.detection.txt"),
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
        let explained_survey = fixture!("heads-up-explained-survey.detection.txt");
        let hidden =
            explained_survey.replacen("on purpose.\n", &format!("on purpose.\n{row}\n"), 1);
        let card = explained_survey.replacen(
            "on purpose.\n",
            "on purpose.\n\n  1: Delete the branch   0: Dismiss\n",
            1,
        );
        let blanks = survey.replacen("0: Dismiss\n", "0: Dismiss\n\n\n\n", 1);
        let hint = format!("{HINT:>210}");
        let card_hint = card.replacen("\n\n\u{2500}", &format!("\n{hint}\n\u{2500}"), 1);
        let cards_gap = explained.replacen("0: Dismiss\n", "0: Dismiss\n\n", 1);
        let cards_hint = explained.replacen("0: Dismiss\n", &format!("0: Dismiss\n\n{hint}"), 1);
        let card_real_hint = card.replacen("\n\n\u{2500}", &format!("\n\n\n{hint}\n\u{2500}"), 1);
        let titled = fixture!("heads-up-titled.detection.txt");
        let untitled = titled.replacen("(2) \u{2500}", "(2)", 1);
        // A box whose rule is not recognized never falls back to a rule and prompt higher up.
        let dashed = titled
            .replacen("review (2)", "review \u{2500} 2", 1)
            .replacen(
                "0: Dismiss\n\n",
                "0: Dismiss\n\n\u{2500}\u{2500}\u{2500}\u{2500}\n\u{276f} Check it\n",
                1,
            );
        let unseen = survey
            .replacen(
                "0: Dismiss\n\n",
                "0: Dismiss\n\n\u{2500}\u{2500} 3 new messages \u{2500}\u{2500}\n\u{276f} Check it\n",
                1,
            )
            .replacen("\n\u{276f}\n", "\n\u{2502} Allow this?\n", 1);
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
            quoted,
            no_box,
            five,
            twice,
            enter,
            order,
            close,
            zero_first,
            no_zero,
            six,
            long,
            format,
            forged,
            no_gap,
            unindented,
            hidden,
            card,
            card_hint,
            cards_gap,
            cards_hint,
            card_real_hint,
            blanks,
            indented,
            between,
            unwrapped,
            replied,
            reply,
            quoted_row,
            untitled,
            dashed,
            unseen,
        ]) {
            assert!(notice(&screen).is_empty(), "{screen}");
        }
    }
}
