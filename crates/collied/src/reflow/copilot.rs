use protocol::TerminalPromptReflow;

use super::plain_text;

const SHADE: &str = "\u{1b}[48;2;20;27;34m";
const BORDER: &str = "\u{1b}[38;2;20;27;34m";
const CLOCK: &str = "\u{1b}[0m\u{1b}[38;2;145;152;161m\u{1b}[48;2;20;27;34m";
const PADDING: &str = "\u{1b}[0m\u{1b}[48;2;20;27;34m  \u{1b}[0m ";

struct Row<'a> {
    raw: &'a str,
    text: String,
    start: usize,
    end: usize,
    clock: Option<String>,
}

pub fn prompts(ansi: &str, originals: &[String]) -> Vec<TerminalPromptReflow> {
    let rows: Vec<_> = ansi.split('\n').collect();
    let mut out = Vec::new();
    let mut top = 0;
    while top < rows.len() {
        if !border(rows[top], '▄') {
            top += 1;
            continue;
        }
        let start = top + 1;
        let mut end = start;
        while end < rows.len() && !border(rows[end], '▀') {
            end += 1;
        }
        if end == rows.len() {
            break;
        }
        let parsed: Option<Vec<_>> = rows[start..end]
            .iter()
            .enumerate()
            .map(|(i, row)| parse(row, i == 0))
            .collect();
        if let Some(parsed) = parsed {
            let mut matched = None;
            let mut ambiguous = false;
            for original in originals {
                if let Some(reflowed) = rejoin(&parsed, original) {
                    if matched.as_ref().is_some_and(|prior| prior != &reflowed) {
                        ambiguous = true;
                        break;
                    }
                    matched = Some(reflowed);
                }
            }
            if !ambiguous && let Some((ansi, true)) = matched {
                out.push(TerminalPromptReflow {
                    start: start as u32,
                    end: end as u32,
                    ansi,
                });
            }
        }
        top = end + 1;
    }
    out
}

fn border(row: &str, block: char) -> bool {
    if !row.contains(BORDER) {
        return false;
    }
    let mut text = String::new();
    plain_text(row, &mut text);
    let text = text.trim();
    !text.is_empty() && text.chars().all(|c| c == block)
}

fn parse(raw: &str, first: bool) -> Option<Row<'_>> {
    if !raw.contains(SHADE) {
        return None;
    }
    let mut text = String::new();
    plain_text(raw, &mut text);
    let prefix = if first { "  ❯ " } else { "    " };
    if !text.starts_with(prefix) {
        return None;
    }
    let mut end = text.trim_end_matches(' ').len();
    let mut clock = None;
    if first {
        let at = raw.rfind(CLOCK)?;
        let mut tail = String::new();
        plain_text(&raw[at..], &mut tail);
        let time = tail.trim_end_matches(' ');
        let bytes = time.as_bytes();
        if bytes.len() != 5
            || bytes[2] != b':'
            || !bytes[..2].iter().chain(&bytes[3..]).all(u8::is_ascii_digit)
            || &bytes[..2] > b"23".as_slice()
            || &bytes[3..] > b"59".as_slice()
        {
            return None;
        }
        let from = end.checked_sub(5)?;
        clock = Some(segment(raw, from, end));
        end = text[..from].trim_end_matches(' ').len();
    } else if raw.contains(CLOCK) {
        return None;
    }
    let start = prefix.len();
    end = end.max(start);
    Some(Row {
        raw,
        text: text.get(start..end)?.to_owned(),
        start,
        end,
        clock,
    })
}

fn rejoin(rows: &[Row<'_>], original: &str) -> Option<(String, bool)> {
    let first = rows.first()?;
    let mut rest = original;
    let mut out = String::new();
    let mut joined = false;
    for (i, row) in rows.iter().enumerate() {
        let mut seam = "";
        if i > 0 {
            seam = if !row.text.is_empty() && rest.starts_with(&row.text) {
                ""
            } else {
                let mut end = rest.find(|c| c != ' ' && c != '\t').unwrap_or(rest.len());
                if rest[end..].starts_with("\r\n") {
                    end += 2;
                } else if rest[end..].starts_with('\n') {
                    end += 1;
                }
                &rest[..end]
            };
            if row.text.is_empty() && !seam.contains('\n') {
                return None;
            }
            rest = &rest[seam.len()..];
        }
        rest = rest.strip_prefix(&row.text)?;
        let newline = seam.contains('\n');
        if newline {
            out.push_str(seam.trim_end_matches(['\r', '\n']));
            out.push_str(PADDING);
            out.push_str("\r\n");
        } else {
            out.push_str(seam);
            joined |= i > 0;
        }
        out.push_str(&segment(
            row.raw,
            if i == 0 || newline { 0 } else { row.start },
            row.end,
        ));
    }
    if !rest.chars().all(|c| c == ' ' || c == '\t') {
        return None;
    }
    out.push_str(rest);
    out.push(' ');
    out.push_str(first.clock.as_ref()?);
    out.push_str(PADDING);
    Some((out, joined))
}

fn segment(raw: &str, start: usize, end: usize) -> String {
    let mut out = String::new();
    let mut plain = 0;
    let mut chars = raw.char_indices();
    while let Some((at, c)) = chars.next() {
        if plain >= end {
            break;
        }
        if c == '\u{1b}' {
            let last = chars.by_ref().find(|(_, c)| *c == 'm');
            if let Some((last, c)) = last {
                out.push_str(&raw[at..last + c.len_utf8()]);
            }
        } else if c != '\r' {
            if plain >= start {
                out.push(c);
            }
            plain += c.len_utf8();
        }
    }
    out
}
