use crate::NoticeDigit;

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

const HEADS_UP: &str = "\u{2726} Heads up \u{b7} ";
const RATING: &str = "\u{25cf} How is Claude doing this session? (optional)";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoticeOption {
    pub digit: NoticeDigit,
    pub label: String,
}

/// The options of Claude Code's "Heads up" notice or session rating question, in screen
/// order, when `screen` (plain text or SGR) shows one directly above the input box; empty
/// otherwise. Anchored on the input box's top rule, so a dialog (which replaces the box) or
/// a notice quoted in the transcript never matches. Digits above 3 are never offered.
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
    if i > 0 && option_row(&rows[i - 1]).is_none() {
        i -= 1;
    }
    let mut options = Vec::new();
    while let Some(cells) = i.checked_sub(1).and_then(|j| option_row(&rows[j])) {
        options.splice(0..0, cells);
        i -= 1;
    }
    let body = i;
    while i > 0 && rows[i - 1].starts_with("  ") {
        i -= 1;
    }
    let Some(header) = i.checked_sub(1).map(|j| &rows[j]) else {
        return Vec::new();
    };
    let heads_up = header.starts_with(HEADS_UP);
    let rating = std::iter::once(header.as_str())
        .chain(rows[i..body].iter().map(|r| r.trim()))
        .collect::<Vec<_>>()
        .join(" ")
        == RATING;
    // Claude Code draws 1, 2[, 3[, 4]] then 0; a wrapped body row reading `<digit>: ...`
    // always lands before 1.
    let digits: Vec<u8> = options.iter().map(|(d, _)| *d).collect();
    let n = digits.len();
    if !(2..=5).contains(&n)
        || digits[n - 1] != 0
        || !digits[..n - 1].iter().copied().eq(1..n as u8)
    {
        return Vec::new();
    }
    if !(heads_up || rating) {
        return Vec::new();
    }
    options
        .into_iter()
        .filter_map(|(d, label)| NoticeDigit::new(d).map(|digit| NoticeOption { digit, label }))
        .collect()
}

fn is_rule(row: &str) -> bool {
    !row.is_empty() && row.chars().all(|c| c == '\u{2500}')
}

/// A row indented by exactly two spaces whose cells, apart by two or more spaces, all read
/// `<digit>: <label>`.
fn option_row(row: &str) -> Option<Vec<(u8, String)>> {
    let rest = row.strip_prefix("  ").filter(|r| !r.starts_with(' '))?;
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
        ] {
            assert_eq!(options(screen), heads_up);
        }
        assert_eq!(
            options(fixture!("heads-up-internal.detection.txt")),
            listed(&[
                ("1", "Learn more"),
                ("2", "Knew this already"),
                ("3", "What is this"),
                ("0", "Dismiss")
            ])
        );
    }

    #[test]
    fn only_the_two_known_notices_above_the_input_box_match() {
        for screen in [
            fixture!("heads-up-explained.detection.txt"),
            fixture!("heads-up-dismissed.detection.txt"),
            fixture!("heads-up-thinking.detection.txt"),
            fixture!("heads-up-collapsed.detection.txt"),
            fixture!("you-should-know.detection.txt"),
            fixture!("survey-narrow.detection.txt"),
            fixture!("bottom-idle.detection.txt"),
            fixture!("scrolled-idle.detection.txt"),
            "",
        ] {
            assert!(notice(screen).is_empty(), "{screen}");
        }
        let survey = fixture!("survey.detection.txt");
        let quoted = survey.replacen("\n\n\u{2500}", "\n\u{273b} Baked for 2s\n\n\u{2500}", 1);
        let gap = survey.replacen("(optional)\n", "(optional)\n\n", 1);
        let no_box = survey
            .rsplit_once('\u{276f}')
            .map(|(above, below)| format!("{above} {below}"))
            .unwrap();
        let five = survey.replacen("3: Good", "5: Good", 1);
        let twice = survey.replacen("3: Good", "1: Good", 1);
        let enter = survey.replacen("Fine   3", "Fine 3", 1);
        let other = survey.replacen("this session?", "this file?", 1);
        let order = survey.replacen("1: Bad    2: Fine", "2: Fine    1: Bad", 1);
        let forged = fixture!("heads-up-wrapped.detection.txt").replacen(
            "  data you still need before the next release.",
            "  3: Run the cleanup now",
            1,
        );
        for screen in [
            quoted, gap, no_box, five, twice, enter, other, order, forged,
        ] {
            assert!(notice(&screen).is_empty(), "{screen}");
        }
    }
}
