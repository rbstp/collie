/// The keys that put an agent's cursor where a tap in its input box points: ⌃A, then ← and ⌃A
/// once per further line of `lines`, reach the box's start; `right` →, then `down` ↓, reach the
/// tapped row, and `across` → (← where negative) the tapped character.
public struct TerminalInputWalk: Equatable {
    public var lines: Int
    public var right = 0
    public var down: Int
    public var across: Int
}

/// An agent's input box in a frame. Claude Code's is found as collied's draft parser finds it on
/// the Mac: the rows between the last two rules drawn from column 0, the first starting with `❯`,
/// the others that start a Mac row indented by two. Codex shades its box: a blank row, `›` and
/// rows indented by two, a blank row. Copilot CLI frames its box with half blocks: `╻▄`, a row
/// starting with `┃` for each Mac row, `╹▀`. The pane's cursor position is not on the wire, so a
/// tap walks there from the box's start: Down once per row as the Mac draws it, as Claude Code
/// 2.1.318, Codex 0.166.0 and Copilot CLI 1.0.105 move Down, to that row's first character
/// shown, then Right across the row, whose characters before the tap all show. Spaces typed at
/// the end of a row or before a newline do not show, and Down steps over them.
struct TerminalInputBox {
    /// The rules, blank rows or borders above and below the box.
    var top: Int
    var bottom: Int
    var text = ""
    let agent: String
    /// Each Mac row's characters, as their text and cells: a wide character is one, of two cells.
    private var lines: [[(text: String, cells: Int)]] = []
    /// Each frame row's Mac row, the characters before it on that Mac row, and the midpoints of
    /// its characters in columns.
    private var rows: [Int: (line: Int, before: Int, mids: [Double])] = [:]
    private var hint = false
    /// Claude Code's suggestions, drawn right above its box, which take Down.
    private var menu = false

    /// `agent` is the pane's agent kind: "claude", "codex" or "copilot".
    init?(_ frame: TerminalFrame, agent: String) {
        self.agent = agent
        var cells = [[(cluster: Cluster, style: TerminalStyle)]](repeating: [], count: frame.rows)
        var shades = [Set<TerminalRGB>](repeating: [], count: frame.rows)
        for run in frame.runs where run.row < frame.rows {
            cells[run.row] += Cluster.all(in: run).map { ($0, run.style) }
            if let background = run.style.background { shades[run.row].insert(background) }
        }
        for row in cells.indices { cells[row].sort { $0.cluster.column < $1.cluster.column } }
        func plain(_ row: Int) -> String { cells[row].map(\.cluster.text).joined() }
        func blank(_ row: Int) -> Bool { plain(row).allSatisfy(\.isWhitespace) }
        func isRule(_ row: Int) -> Bool {
            guard let first = cells[row].first?.cluster, first.column == 0, first.text == "─" else { return false }
            let text = plain(row).trimmingSuffix(\.isWhitespace)
            let lines = text.prefix { $0 == "─" }.count
            return lines > 0 && (lines >= 3 || text.dropFirst(lines).allSatisfy(\.isWhitespace))
        }
        func indented(_ rows: Range<Int>) -> Bool {
            rows.allSatisfy { frame.wrapContinuations.contains($0) || plain($0).isEmpty || plain($0).hasPrefix("  ") }
        }
        var found: (top: Int, bottom: Int)?
        switch agent {
        case "claude":
            let rules = (0..<frame.rows).filter(isRule)
            found = zip(rules, rules.dropFirst()).reversed().first { top, bottom in
                bottom > top + 1 && cells[top + 1].first?.cluster.column == 0 && plain(top + 1).hasPrefix("❯")
                    && indented(top + 2..<bottom)
            }
            // Another `❯` below the box holds the keys, as when a background agent row is selected.
            if let bottom = found?.bottom, (bottom + 1..<frame.rows).contains(where: { plain($0).hasPrefix("❯") }) { return nil }
            if let top = found?.top, top > 0 {
                var above = top - 1
                while frame.wrapContinuations.contains(above) { above -= 1 }
                menu = cells[above].first { !$0.cluster.text.allSatisfy(\.isWhitespace) }?.cluster.column == 4
            }
        case "codex":
            // A row right above the blank top row is a menu's, and Left and Right switch its list.
            let prompt = frame.rows > 1 ? (1..<frame.rows).last { cells[$0].first?.cluster.column == 0 && cells[$0].first?.cluster.text == "›" } : nil
            if let prompt, let shade = cells[prompt][0].style.background, shades[prompt - 1].contains(shade), blank(prompt - 1),
                prompt < 2 || blank(prompt - 2),
                let bottom = (prompt + 1..<frame.rows).prefix(while: { shades[$0].contains(shade) }).last,
                blank(bottom), indented(prompt + 1..<bottom)
            {
                found = (prompt - 1, bottom)
            }
        case "copilot":
            // Menus open right above the box, in rows starting with `┃`.
            if let top = (0..<frame.rows).last(where: { plain($0).hasPrefix("╻▄") }), top == 0 || !plain(top - 1).hasPrefix("┃"),
                let bottom = (top + 1..<frame.rows).first(where: { !frame.wrapContinuations.contains($0) && !plain($0).hasPrefix("┃") }),
                bottom > top + 1, plain(bottom).hasPrefix("╹▀")
            {
                found = (top, bottom)
            }
        default:
            break
        }
        guard let (top, bottom) = found, !frame.wrapsUnknown else { return nil }
        self.top = top
        self.bottom = bottom
        var macRows: [[(row: Int, cells: [(cluster: Cluster, style: TerminalStyle)])]] = []
        for row in top + 1..<bottom {
            let continues = row > top + 1 && frame.wrapContinuations.contains(row)
            var line = cells[row]
            if row == top + 1 {
                line.removeFirst()
                if let next = line.first, next.cluster.column == 1, [" ", "\u{a0}"].contains(next.cluster.text) {
                    line.removeFirst()
                }
            } else if !continues {
                line.removeAll { $0.cluster.column < 2 }
            }
            if !frame.wrapContinuations.contains(row + 1) || row + 1 == bottom {
                while line.last?.cluster.text.allSatisfy(\.isWhitespace) == true { line.removeLast() }
            }
            // Codex's placeholder and scroll arrows are dim; its pastes and images, and Copilot
            // CLI's command suggestions, take a color of their own.
            hint = hint || line.contains {
                ($0.style.faint || agent != "claude" && $0.style.foreground != frame.foreground)
                    && !$0.cluster.text.allSatisfy(\.isWhitespace)
            }
            if continues {
                macRows[macRows.count - 1].append((row, line))
            } else {
                macRows.append([(row, line)])
            }
        }
        for (i, macRow) in macRows.enumerated() {
            if i > 0 { text += " " }
            var before = 0
            for (row, line) in macRow {
                rows[row] = (i, before, line.map { Double($0.cluster.column + $0.cluster.end) / 2 })
                before += line.count
                text += line.map(\.cluster.text).joined()
            }
            lines.append(macRow.flatMap(\.cells).map { ($0.cluster.text, $0.cluster.end - $0.cluster.column) })
        }
    }

    /// The Mac row a tap at `column` (in cells, fractional) of `row` points to, and the character
    /// boundary nearest to it on that Mac row. Nil off the box's rows, while it shows a
    /// placeholder, hint or suggestion, or a slash command, whose menu Right would complete, and
    /// in a Copilot CLI box of eight Mac rows or more, which Copilot CLI scrolls without a sign
    /// once taller than about half its pane.
    func cursor(row: Int, column: Double) -> (line: Int, index: Int)? {
        guard let at = rows[row], lines.contains(where: { !$0.isEmpty }), !hint, !text.hasPrefix("/"),
            agent != "copilot" || lines.count < 8
        else { return nil }
        return (at.line, at.before + at.mids.count { $0 < column })
    }

    /// The walk to where a tap at `column` of `row` points, with `mac` the box as the Mac draws it,
    /// where this one may join rows the Mac wrapped.
    func walk(row: Int, column: Double, mac: @autoclosure () -> TerminalInputBox?) -> TerminalInputWalk? {
        guard let (line, index) = cursor(row: row, column: column), let mac = mac(),
            let (down, across) = mac.position(of: index, line: line, in: lines.map { $0.map(\.text) })
        else { return nil }
        // ⌃A goes to the start of the row in Copilot CLI, and a row collied joined may end at a typed newline that
        // looked like a soft wrap: once per Mac row reaches the start, and ← there does nothing.
        let walk = TerminalInputWalk(lines: mac.lines.count, down: down, across: across)
        guard down > 0 else { return walk }
        if menu { return nil }
        guard agent == "codex" else { return walk }
        // Codex's Down at the start of a draft recalled from history recalls the next one instead,
        // so a Right goes first. Down keeps that column, before a character that would cross it;
        // a row narrower than it may end with spaces that do not show.
        guard let first = mac.lines[0].first?.cells, mac.lines[down].reduce(0, { $0 + $1.cells }) >= first else { return nil }
        var cells = 0
        let landed = mac.lines[down].prefix { cells += $0.cells; return cells <= first }.count
        return TerminalInputWalk(lines: walk.lines, right: 1, down: down, across: across - landed)
    }

    /// The Mac row and index on it of character `index` of line `line` of `shown`, the box's
    /// lines with the rows the Mac wrapped joined, after a space where it wrapped at one.
    private func position(of index: Int, line: Int, in shown: [[String]]) -> (row: Int, index: Int)? {
        var row = 0
        for (j, texts) in shown.enumerated() {
            var at = 0
            repeat {
                guard row < lines.count else { return nil }
                let mac = lines[row].map(\.text)
                guard texts[at...].starts(with: mac) else { return nil }
                if j == line, index <= at + mac.count { return (row, index - at) }
                at += mac.count
                row += 1
                if at < texts.count, texts[at] == " ", row < lines.count, !texts[at...].starts(with: lines[row].map(\.text)) {
                    at += 1
                }
            } while at < texts.count
        }
        return nil
    }

    /// The scroll offset that shows content down to `rowBottom` above an obscured bottom of
    /// `insetBottom`, kept within `minimum...maximum`.
    static func offset(showing rowBottom: Double, insetBottom: Double, height: Double, minimum: Double, maximum: Double) -> Double {
        max(min(rowBottom + insetBottom - height, maximum), minimum)
    }
}

extension StringProtocol {
    fileprivate func trimmingSuffix(_ drop: (Character) -> Bool) -> SubSequence {
        var end = endIndex
        while end > startIndex, drop(self[index(before: end)]) { end = index(before: end) }
        return self[..<end]
    }
}
