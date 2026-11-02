use std::time::Instant;

use alacritty_terminal::event::VoidListener;
use alacritty_terminal::grid::{Dimensions, Row};
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::cell::{Cell, Flags};
use alacritty_terminal::term::{Config, Term};
use alacritty_terminal::vte::ansi::{Color, NamedColor, Processor};

/// A herdr-core terminal emulated from its raw output, read back as herdr reads a pane: rows
/// joined by `\r\n`, each style run opened with `ESC[0m` and one SGR per attribute, as
/// herdr-core paints them.
pub struct Screen {
    term: Term<VoidListener>,
    parser: Processor,
}

struct Size(usize, usize);

impl Dimensions for Size {
    fn total_lines(&self) -> usize {
        self.1
    }

    fn screen_lines(&self) -> usize {
        self.1
    }

    fn columns(&self) -> usize {
        self.0
    }
}

const STYLE: Flags = Flags::BOLD
    .union(Flags::DIM)
    .union(Flags::ITALIC)
    .union(Flags::ALL_UNDERLINES)
    .union(Flags::INVERSE)
    .union(Flags::HIDDEN)
    .union(Flags::STRIKEOUT);

type Style = (Flags, Color, Color);

/// Bounds what a core's answer can make collied allocate: wider or taller terminals are
/// emulated cut down to this.
const MAX_COLS: u16 = 1000;
const MAX_ROWS: u16 = 500;

impl Screen {
    pub fn new(cols: u16, rows: u16, history: usize) -> Self {
        let config = Config {
            scrolling_history: history,
            ..Config::default()
        };
        let size = Size(
            usize::from(cols.clamp(1, MAX_COLS)),
            usize::from(rows.clamp(1, MAX_ROWS)),
        );
        Self {
            term: Term::new(config, &size, VoidListener),
            parser: Processor::new(),
        }
    }

    pub fn feed(&mut self, bytes: &[u8]) {
        self.parser.advance(&mut self.term, bytes);
    }

    /// `visible`: the screen's rows. Otherwise the last `lines` rows of history and screen,
    /// with rows the terminal soft-wrapped joined into one, and no blank rows at the top.
    /// Blank rows at the bottom are left out. A row wrapped before the emulator attached
    /// arrives as rows: a paint draws each row on its own.
    pub fn read(&mut self, source: &str, lines: Option<u32>) -> String {
        // A synchronized update left open past its deadline is drawn, as a terminal would.
        if self
            .parser
            .sync_timeout()
            .sync_timeout()
            .is_some_and(|at| Instant::now() >= at)
        {
            self.parser.stop_sync(&mut self.term);
        }
        let grid = self.term.grid();
        let visible = source == "visible";
        let top = if visible {
            0
        } else {
            -(grid.history_size() as i32)
        };
        let mut rows: Vec<&Row<Cell>> = (top..grid.screen_lines() as i32)
            .map(|l| &grid[Line(l)])
            .collect();
        while rows.last().is_some_and(|r| blank(r)) {
            rows.pop();
        }
        if !visible {
            // Clearing an empty screen, as every paint starts with, pushes a blank row into
            // alacritty's history.
            let first = rows.iter().position(|r| !blank(r)).unwrap_or(rows.len());
            let keep = lines.map_or(rows.len(), |n| n as usize);
            rows.drain(..first.max(rows.len().saturating_sub(keep)));
        }
        let mut out = Vec::new();
        let mut line: Vec<&Cell> = Vec::new();
        for row in rows {
            let cols = row.len();
            line.extend((0..cols).map(|c| &row[Column(c)]));
            if visible || !row[Column(cols - 1)].flags.contains(Flags::WRAPLINE) {
                out.push(render(&line));
                line.clear();
            }
        }
        if !line.is_empty() {
            out.push(render(&line));
        }
        out.join("\r\n")
    }
}

fn style(cell: &Cell) -> Style {
    (cell.flags & STYLE, cell.fg, cell.bg)
}

fn plain(cell: &Cell) -> bool {
    (cell.c == ' ' || cell.flags.contains(Flags::LEADING_WIDE_CHAR_SPACER))
        && cell.zerowidth().is_none()
        && sgr(style(cell)).is_empty()
}

fn blank(row: &Row<Cell>) -> bool {
    (0..row.len()).all(|c| plain(&row[Column(c)]))
}

fn push_symbol(out: &mut String, cell: &Cell) {
    if cell
        .flags
        .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
    {
        return;
    }
    out.push(if cell.c == '\t' { ' ' } else { cell.c });
    out.extend(cell.zerowidth().into_iter().flatten());
}

/// One line without the unstyled blanks after its last character.
fn render(cells: &[&Cell]) -> String {
    let end = cells.iter().rposition(|c| !plain(c)).map_or(0, |i| i + 1);
    let mut out = String::new();
    let mut current: Option<Style> = None;
    let mut open = false;
    for cell in &cells[..end] {
        if cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
            continue;
        }
        let s = style(cell);
        if current != Some(s) {
            let codes = sgr(s);
            if open || !codes.is_empty() {
                out.push_str("\u{1b}[0m");
                out.push_str(&codes);
            }
            open = !codes.is_empty();
            current = Some(s);
        }
        push_symbol(&mut out, cell);
    }
    if open {
        out.push_str("\u{1b}[0m");
    }
    out
}

fn sgr((flags, fg, bg): Style) -> String {
    let mut out = String::new();
    for (flag, code) in [
        (Flags::BOLD, 1),
        (Flags::DIM, 2),
        (Flags::ITALIC, 3),
        (Flags::ALL_UNDERLINES, 4),
        (Flags::INVERSE, 7),
        (Flags::HIDDEN, 8),
        (Flags::STRIKEOUT, 9),
    ] {
        if flags.intersects(flag) {
            out.push_str(&format!("\u{1b}[{code}m"));
        }
    }
    for (color, layer) in [(fg, 38), (bg, 48)] {
        match color {
            Color::Spec(c) => out.push_str(&format!("\u{1b}[{layer};2;{};{};{}m", c.r, c.g, c.b)),
            Color::Indexed(i) => out.push_str(&format!("\u{1b}[{layer};5;{i}m")),
            Color::Named(n) => {
                if let Some(i) = palette(n) {
                    out.push_str(&format!("\u{1b}[{layer};5;{i}m"));
                }
            }
        }
    }
    out
}

/// herdr-core writes the sixteen named colors as palette indexes; the default foreground and
/// background have none.
fn palette(named: NamedColor) -> Option<u8> {
    use NamedColor::*;
    Some(match named {
        Black | DimBlack => 0,
        Red | DimRed => 1,
        Green | DimGreen => 2,
        Yellow | DimYellow => 3,
        Blue | DimBlue => 4,
        Magenta | DimMagenta => 5,
        Cyan | DimCyan => 6,
        White | DimWhite => 7,
        BrightBlack => 8,
        BrightRed => 9,
        BrightGreen => 10,
        BrightYellow => 11,
        BrightBlue => 12,
        BrightMagenta => 13,
        BrightCyan => 14,
        BrightWhite => 15,
        Foreground | Background | Cursor | BrightForeground | DimForeground => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn screen(cols: u16, rows: u16, bytes: &str) -> Screen {
        let mut s = Screen::new(cols, rows, 100);
        s.feed(bytes.as_bytes());
        s
    }

    #[test]
    fn styles_read_back_as_herdr_core_paints_them() {
        let mut s = screen(
            40,
            4,
            "\x1b[1;31mred\x1b[0m plain \x1b[38;5;208mx\x1b[48;2;1;2;3mbg\x1b[0m w\r\n\
             \x1b[44mbluebg   \x1b[0m   \r\n\
             \x1b[2mdim\x1b[0m \x1b[7mrev\x1b[0m \x1b[3mit\x1b[0m \x1b[4mul\x1b[0m \x1b[9mst\x1b[0m \x1b[8mhid\x1b[0m \x1b[93mbright\x1b[0m \x1b[1;2mbd\x1b[0m\r\n\
             wide \u{6f22}\u{5b57} x",
        );
        // Captured from herdr-core 0.1.16's paint of the same output, except that trailing
        // blanks the program wrote are dropped: the emulator cannot tell them from empty cells.
        assert_eq!(
            s.read("visible", None),
            "\x1b[0m\x1b[1m\x1b[38;5;1mred\x1b[0m plain \x1b[0m\x1b[38;5;208mx\x1b[0m\x1b[38;5;208m\x1b[48;2;1;2;3mbg\x1b[0m w\r\n\
             \x1b[0m\x1b[48;5;4mbluebg   \x1b[0m\r\n\
             \x1b[0m\x1b[2mdim\x1b[0m \x1b[0m\x1b[7mrev\x1b[0m \x1b[0m\x1b[3mit\x1b[0m \x1b[0m\x1b[4mul\x1b[0m \x1b[0m\x1b[9mst\x1b[0m \x1b[0m\x1b[8mhid\x1b[0m \x1b[0m\x1b[38;5;11mbright\x1b[0m \x1b[0m\x1b[1m\x1b[2mbd\x1b[0m\r\n\
             wide \u{6f22}\u{5b57} x"
        );
    }

    #[test]
    fn recent_joins_soft_wraps_and_counts_rows() {
        let mut s = screen(10, 3, "one\r\n0123456789abcde\r\nlast\r\n");
        assert_eq!(s.read("visible", None), "abcde\r\nlast");
        assert_eq!(
            s.read("recent_unwrapped", Some(100)),
            "one\r\n0123456789abcde\r\nlast"
        );
        assert_eq!(s.read("recent_unwrapped", Some(2)), "abcde\r\nlast");
        assert_eq!(
            s.read("recent_unwrapped", Some(3)),
            "0123456789abcde\r\nlast"
        );
    }

    #[test]
    fn a_screen_is_never_larger_than_its_bounds() {
        let s = Screen::new(u16::MAX, u16::MAX, 0);
        assert_eq!(
            (s.term.columns(), s.term.screen_lines()),
            (usize::from(MAX_COLS), usize::from(MAX_ROWS))
        );
    }

    #[test]
    fn a_paint_leaves_no_blank_row_above_the_screen() {
        let mut s = screen(10, 3, "\x1b[H\x1b[2Jready\r\n");
        assert_eq!(s.read("recent_unwrapped", None), "ready");
    }

    #[test]
    fn a_style_runs_across_a_soft_wrap() {
        let mut s = screen(4, 3, "\x1b[32mabcdef\x1b[0m");
        assert_eq!(
            s.read("recent_unwrapped", None),
            "\x1b[0m\x1b[38;5;2mabcdef\x1b[0m"
        );
    }

    #[test]
    fn controls_and_strings_never_reach_the_read() {
        let mut s = screen(
            20,
            2,
            "\x1b]0;title\x07\x1b]8;;https://x\x1b\\link\x1b]8;;\x1b\\\x1b]52;c;aGk=\x07\x1b[6n ok",
        );
        assert_eq!(s.read("visible", None), "link ok");
    }

    /// Each fixture is an agent's whole output on herdr-core 0.1.16, from a watch-only attach
    /// made right after it started through two turns, and the core's `terminal.read` after.
    #[test]
    fn the_emulated_screen_reads_as_the_core_reads_it() {
        for (bytes, read) in [
            (
                &include_bytes!("../../tests/fixtures/herdr-core-0.1.16/claude-2.1.316.bytes")[..],
                include_str!("../../tests/fixtures/herdr-core-0.1.16/claude-2.1.316.read.txt"),
            ),
            (
                include_bytes!("../../tests/fixtures/herdr-core-0.1.16/codex-0.166.0.bytes"),
                include_str!("../../tests/fixtures/herdr-core-0.1.16/codex-0.166.0.read.txt"),
            ),
        ] {
            let mut s = Screen::new(200, 120, 0);
            s.feed(bytes);
            let mut want: Vec<&str> = read.lines().map(str::trim_end).collect();
            while want.last().is_some_and(|l| l.is_empty()) {
                want.pop();
            }
            let got: Vec<String> = s
                .read("visible", None)
                .split("\r\n")
                .map(|row| {
                    let mut text = String::new();
                    let mut rest = row;
                    while let Some(at) = rest.find('\u{1b}') {
                        text.push_str(&rest[..at]);
                        rest = &rest[at + rest[at..].find('m').unwrap() + 1..];
                    }
                    text.push_str(rest);
                    text.trim_end().to_owned()
                })
                .collect();
            assert_eq!(got.join("\n"), want.join("\n"));
        }
    }

    #[test]
    fn an_open_synchronized_update_shows_the_screen_before_it() {
        let mut s = screen(10, 2, "before\x1b[?2026h\x1b[2J\x1b[Hafter");
        assert_eq!(s.read("visible", None), "before");
        s.feed(b"\x1b[?2026l");
        assert_eq!(s.read("visible", None), "after");
    }
}
