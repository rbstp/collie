/// An agent's input box in a frame. Claude Code's is found as collied's draft parser finds it on
/// the Mac: the rows between the last two rules drawn from column 0, the first starting with `❯`,
/// the others that start a Mac row indented by two. Codex shades its box: a blank row, `›` and
/// rows indented by two, a blank row. Copilot CLI frames its box with half blocks: `╻▄`, a row
/// starting with `┃` for each Mac row, `╹▀`. The pane's cursor position is not on the wire, so a
/// tap counts characters in the box instead: a Mac row after a full one adds one for the space
/// it wrapped at, as Claude Code 2.1.318, Codex 0.166.0 and Copilot CLI 1.0.105 move Left across
/// a wrapped row, except where Copilot CLI starts a row with the space after a full one. After a
/// row that is not full, whose spaces typed at the end or before a newline do not show, a full
/// row whose word may continue on the next, or a wide character at either side, the count is
/// unknown, and later rows take no taps.
struct TerminalInputBox {
    /// The rules, blank rows or borders above and below the box.
    var top: Int
    var bottom: Int
    var text = ""
    /// Characters before each row's first one, and the midpoints of that row's characters in
    /// columns: a wide character counts once, centered on its two cells.
    private var rows: [Int: (before: Int, mids: [Double])] = [:]
    private var count = 0
    private var lines = 0
    private var hint = false

    /// `agent` is the pane's agent kind: "claude", "codex" or "copilot"; `snapshot` is what `frame`
    /// was rendered from, where the rows drawn across the Mac pane keep their width.
    init?(_ frame: TerminalFrame, agent: String, snapshot: String) {
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
        lines = macRows.count
        let width = Self.wrapWidth(in: snapshot, agent: agent)
        // Copilot CLI scrolls a box taller than about half its pane, without a sign, so a taller
        // box may start above its first row shown.
        var known = agent != "copilot" || lines < 8
        for (i, macRow) in macRows.enumerated() {
            if i > 0 {
                let (above, below) = (macRows[i - 1].flatMap(\.cells).map(\.cluster), macRow.flatMap(\.cells).map(\.cluster))
                let join = Self.join(above, below, width: width, agent: agent)
                known = known && join != nil
                count += join ?? 1
                text += join == 0 ? "" : " "
            }
            for (row, line) in macRow {
                if known { rows[row] = (count, line.map { Double($0.cluster.column + $0.cluster.end) / 2 }) }
                text += line.map(\.cluster.text).joined()
                count += line.count
            }
        }
    }

    /// Where a tap at `column` (in cells, fractional) of `row` puts the cursor: the character
    /// boundary nearest to it on that row, as `(index, lines)` with `lines` Mac rows in the box.
    /// Nil off the box's rows or past a row the count is unknown after, and while it shows a
    /// placeholder, hint or suggestion, or a slash command, whose menu Right would complete.
    func cursor(row: Int, column: Double) -> (index: Int, lines: Int)? {
        guard let line = rows[row], count > 0, !hint, !text.hasPrefix("/") else { return nil }
        return (line.before + line.mids.count { $0 < column }, lines)
    }

    /// The characters Left crosses from Mac row `below` into the full row `above`: one for the
    /// space `above` ended at, none where Copilot CLI starts `below` with the space after a full
    /// row. Nil where `above` is not full, since spaces typed at its end or before a newline do not
    /// show, where a word may continue on `below`, or where a wide character ends or starts them.
    private static func join(_ above: [Cluster], _ below: [Cluster], width: Int?, agent: String) -> Int? {
        func cells(_ clusters: some Sequence<Cluster>) -> Int { clusters.reduce(0) { $0 + $1.end - $1.column } }
        guard let width, cells(above) == width, let first = below.first, let last = above.last, cells([last]) == 1, cells([first]) == 1
        else { return nil }
        if first.text == " " { return agent == "copilot" ? 0 : nil }
        // A word that fits a row is never split, so these two were not one word.
        return cells(above.reversed().prefix { $0.text != " " }) + cells(below.prefix { $0.text != " " }) <= width ? 1 : nil
    }

    /// The columns the agent wraps its box's text at on the Mac, from rows the snapshot draws
    /// across the pane: Claude Code's last rule less 4, the blank row above Codex's last `›` row
    /// and Copilot CLI's last `╻▄` border less 3.
    static func wrapWidth(in snapshot: String, agent: String) -> Int? {
        let rows = snapshot.split(omittingEmptySubsequences: false, whereSeparator: \.isNewline)
        let plain = rows.map { line in
            var out = String.UnicodeScalarView()
            var escape = false
            for scalar in line.unicodeScalars {
                if scalar == "\u{1B}" {
                    escape = true
                } else if escape {
                    escape = scalar != "m"
                } else {
                    out.append(scalar)
                }
            }
            return String(out).trimmingSuffix { $0 == " " }
        }
        switch agent {
        case "claude":
            return plain.last { !$0.isEmpty && $0.allSatisfy { $0 == "─" } }.map { $0.count - 4 }
        case "codex":
            guard let prompt = plain.lastIndex(where: { $0.hasPrefix("›") }), prompt > 0 else { return nil }
            return rows[prompt - 1].filter { $0 == " " }.count - 3
        case "copilot":
            return plain.last { $0.hasPrefix("╻▄") }.map { $0.count - 3 }
        default:
            return nil
        }
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
