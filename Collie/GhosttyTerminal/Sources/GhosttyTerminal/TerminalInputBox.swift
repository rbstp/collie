/// Claude Code's input box in a frame, found as collied's draft parser finds it on the Mac: the
/// rows between the last two rules drawn from column 0, the first starting with `❯`, the others
/// that start a Mac row indented by two. The pane's cursor position is not on the wire, so a tap
/// counts characters in the box instead: each Mac row after the first adds one for the space or
/// newline it wrapped at, as Claude Code 2.1.318 moves Left across a wrapped row.
struct TerminalInputBox {
    /// The rules above and below the box.
    var top: Int
    var bottom: Int
    var text = ""
    /// Characters before each row's first one, and the midpoints of that row's characters in
    /// columns: a wide character counts once, centered on its two cells.
    private var rows: [Int: (before: Int, mids: [Double])] = [:]
    private var count = 0
    private var faint = false

    init?(_ frame: TerminalFrame) {
        var cells = [[(cluster: Cluster, faint: Bool)]](repeating: [], count: frame.rows)
        for run in frame.runs where run.row < frame.rows {
            cells[run.row] += Cluster.all(in: run).map { ($0, run.style.faint) }
        }
        for row in cells.indices { cells[row].sort { $0.cluster.column < $1.cluster.column } }
        func plain(_ row: Int) -> String { cells[row].map(\.cluster.text).joined() }
        func isRule(_ row: Int) -> Bool {
            guard let first = cells[row].first?.cluster, first.column == 0, first.text == "─" else { return false }
            let text = plain(row).trimmingSuffix(\.isWhitespace)
            let lines = text.prefix { $0 == "─" }.count
            return lines > 0 && (lines >= 3 || text.dropFirst(lines).allSatisfy(\.isWhitespace))
        }
        let rules = (0..<frame.rows).filter(isRule)
        let found = zip(rules, rules.dropFirst()).reversed().first { top, bottom in
            bottom > top + 1 && cells[top + 1].first?.cluster.column == 0 && plain(top + 1).hasPrefix("❯")
                && (top + 2..<bottom).allSatisfy {
                    frame.wrapContinuations.contains($0) || plain($0).isEmpty || plain($0).hasPrefix("  ")
                }
        }
        // Another `❯` below the box holds the keys, as when a background agent row is selected.
        guard let (top, bottom) = found, !frame.wrapsUnknown,
            !(bottom + 1..<frame.rows).contains(where: { plain($0).hasPrefix("❯") })
        else { return nil }
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
            faint = faint || line.contains { $0.faint && !$0.cluster.text.allSatisfy(\.isWhitespace) }
            rows[row] = (count, line.map { Double($0.cluster.column + $0.cluster.end) / 2 })
            text += line.map(\.cluster.text).joined()
            count += line.count
        }
    }

    /// Where a tap at `column` (in cells, fractional) of `row` puts the cursor: the character
    /// boundary nearest to it on that row, as `(index, count)` with `count` characters in the box.
    /// Nil off the box's rows, and while it shows only Claude Code's dim placeholder or hint, or
    /// a slash command, whose menu Right would complete.
    func cursor(row: Int, column: Double) -> (index: Int, count: Int)? {
        guard let line = rows[row], count > 0, !faint, !text.hasPrefix("/") else { return nil }
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
