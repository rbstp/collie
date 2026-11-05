/// An agent's input box in a frame. Claude Code's is found as collied's draft parser finds it on
/// the Mac: the rows between the last two rules drawn from column 0, the first starting with `❯`,
/// the others that start a Mac row indented by two. Codex shades its box: a blank row, `›` and
/// rows indented by two, a blank row. Copilot CLI frames its box with half blocks: `╻▄`, a row
/// starting with `┃` for each Mac row, `╹▀`. The pane's cursor position is not on the wire, so a
/// tap counts characters in the box instead: each Mac row after the first adds one for the space
/// or newline it wrapped at, as Claude Code 2.1.318, Codex 0.166.0 and Copilot CLI 1.0.105 move
/// Left across a wrapped row.
struct TerminalInputBox {
    /// The rules, blank rows or borders above and below the box.
    var top: Int
    var bottom: Int
    var text = ""
    /// Characters before each row's first one, and the midpoints of that row's characters in
    /// columns: a wide character counts once, centered on its two cells.
    private var rows: [Int: (before: Int, mids: [Double])] = [:]
    private var count = 0
    private var hint = false

    /// `agent` is the pane's agent kind: "claude", "codex" or "copilot".
    init?(_ frame: TerminalFrame, agent: String) {
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
        for row in top + 1..<bottom {
            let continues = row > top + 1 && frame.wrapContinuations.contains(row)
            if row > top + 1 && !continues {
                count += 1
                text += " "
            }
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
            rows[row] = (count, line.map { Double($0.cluster.column + $0.cluster.end) / 2 })
            text += line.map(\.cluster.text).joined()
            count += line.count
        }
    }

    /// Where a tap at `column` (in cells, fractional) of `row` puts the cursor: the character
    /// boundary nearest to it on that row, as `(index, count)` with `count` characters in the box.
    /// Nil off the box's rows, and while it shows a placeholder, hint or suggestion, or a slash
    /// command, whose menu Right would complete.
    func cursor(row: Int, column: Double) -> (index: Int, count: Int)? {
        guard let line = rows[row], count > 0, !hint, !text.hasPrefix("/") else { return nil }
        return (line.before + line.mids.count { $0 < column }, count)
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
