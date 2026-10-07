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
}
