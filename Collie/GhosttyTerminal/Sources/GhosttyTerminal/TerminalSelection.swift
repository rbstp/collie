import GhosttyVt

struct TerminalCell: Comparable, Hashable, Sendable {
    var row: Int
    var column: Int

    static func < (a: TerminalCell, b: TerminalCell) -> Bool {
        (a.row, a.column) < (b.row, b.column)
    }

    /// The cell under a point in content coordinates. Points above the content map to its first
    /// cell, points below it to its last, and columns clamp to the row.
    static func at(
        x: Double, y: Double, cellWidth: Double, cellHeight: Double, inset: Double, in frame: TerminalFrame
    ) -> TerminalCell? {
        guard frame.rows > 0, frame.columns > 0, cellWidth > 0, cellHeight > 0, x.isFinite, y.isFinite else { return nil }
        let row = ((y - inset) / cellHeight).rounded(.down)
        if row < 0 { return TerminalCell(row: 0, column: 0) }
        if row >= Double(frame.rows) { return TerminalCell(row: frame.rows - 1, column: frame.columns - 1) }
        let column = min(max(((x - inset) / cellWidth).rounded(.down), 0), Double(frame.columns - 1))
        return TerminalCell(row: Int(row), column: Int(column))
    }
}

/// Cells from `start` to `end` in reading order, both included, like a terminal selection.
struct TerminalSelection: Equatable, Sendable {
    enum Edge: Sendable {
        case start, end
    }

    private(set) var start: TerminalCell
    private(set) var end: TerminalCell

    init(_ a: TerminalCell, _ b: TerminalCell) {
        start = min(a, b)
        end = max(a, b)
    }

    /// The run of non-space characters under `cell`, or that cell alone when it holds a space,
    /// box drawing, or a run without letters or digits. A wide character's two cells count as one.
    static func word(at cell: TerminalCell, in frame: TerminalFrame) -> TerminalSelection {
        var clusters: [Int: Cluster] = [:]
        for run in frame.runs where run.row == cell.row {
            for cluster in Cluster.all(in: run) {
                for column in cluster.column..<cluster.end { clusters[column] = cluster }
            }
        }
        func inWord(_ cluster: Cluster?) -> Bool {
            guard let scalar = cluster?.text.unicodeScalars.first else { return false }
            return !scalar.properties.isWhitespace && !(0x2500...0x259F).contains(scalar.value)
        }
        guard let hit = clusters[cell.column], inWord(hit) else { return TerminalSelection(cell, cell) }
        var first = hit
        var last = hit
        while let previous = clusters[first.column - 1], inWord(previous) { first = previous }
        while let next = clusters[last.end], inWord(next) { last = next }
        let hasText = (first.column..<last.end).contains { column in
            clusters[column]?.text.unicodeScalars.contains { $0.properties.isAlphabetic || $0.properties.numericType != nil } == true
        }
        if !hasText { (first, last) = (hit, hit) }
        return TerminalSelection(
            TerminalCell(row: cell.row, column: first.column), TerminalCell(row: cell.row, column: last.end - 1)
        )
    }

    /// This selection grown to reach `cell`, as a drag from a selected word does.
    func extended(to cell: TerminalCell) -> TerminalSelection {
        TerminalSelection(min(start, cell), max(end, cell))
    }

    /// The selection with `edge` moved to `cell`, and the edge the drag now holds: the ends swap
    /// when they cross.
    func moving(_ edge: Edge, to cell: TerminalCell) -> (selection: TerminalSelection, edge: Edge) {
        let fixed = edge == .start ? end : start
        let moved = cell < fixed ? Edge.start : cell > fixed ? .end : edge
        return (TerminalSelection(fixed, cell), moved)
    }

    /// Selected columns of `row`, both included, in a frame `columns` wide.
    func columns(inRow row: Int, columns: Int) -> ClosedRange<Int>? {
        guard row >= start.row, row <= end.row, columns > 0 else { return nil }
        let first = row == start.row ? start.column : 0
        let last = min(row == end.row ? end.column : columns - 1, columns - 1)
        return first <= last ? first...last : nil
    }

    /// The selection kept for a new frame, or nil when its rows are gone.
    func clamped(to frame: TerminalFrame) -> TerminalSelection? {
        guard end.row < frame.rows, frame.columns > 0 else { return nil }
        func clamp(_ cell: TerminalCell) -> TerminalCell {
            TerminalCell(row: cell.row, column: min(cell.column, frame.columns - 1))
        }
        return TerminalSelection(clamp(start), clamp(end))
    }

    /// The selected text: a wide character partly selected is copied once, blank columns read as
    /// spaces, trailing spaces are dropped at each line end, and rows that continue a soft wrap
    /// join the row above without a newline, keeping a space the wrap fell on.
    func text(in frame: TerminalFrame) -> String {
        guard start.row < frame.rows else { return "" }
        let rows = Dictionary(grouping: frame.runs, by: \.row)
        var text = ""
        for row in start.row...min(end.row, frame.rows - 1) {
            guard let selected = columns(inRow: row, columns: frame.columns) else { continue }
            if row > start.row && !frame.wrapContinuations.contains(row) {
                text.trimTrailingSpaces()
                text += "\n"
            }
            var next = selected.lowerBound
            for run in (rows[row] ?? []).sorted(by: { $0.startColumn < $1.startColumn }) {
                for cluster in Cluster.all(in: run) where cluster.column <= selected.upperBound && cluster.end > selected.lowerBound {
                    if cluster.column > next { text += String(repeating: " ", count: cluster.column - next) }
                    text += cluster.text
                    next = max(next, cluster.end)
                }
            }
        }
        text.trimTrailingSpaces()
        return text
    }
}

/// One cell's characters and the columns they cover: two for a wide character.
private struct Cluster {
    var column: Int
    var end: Int
    var text: String

    static func all(in run: TerminalRun) -> [Cluster] {
        let units = Array(run.text.utf16)
        guard units.count == run.utf16Columns.count else { return [] }
        var clusters: [Cluster] = []
        var i = 0
        while i < units.count {
            let column = run.utf16Columns[i]
            var j = i + 1
            while j < units.count && run.utf16Columns[j] == column { j += 1 }
            let text = String(decoding: units[i..<j], as: UTF16.self)
            let width = text.unicodeScalars.first.map { max(Int(ghostty_unicode_codepoint_width($0.value)), 1) } ?? 1
            let limit = j < units.count ? run.utf16Columns[j] : run.endColumn
            clusters.append(Cluster(column: column, end: max(min(column + width, limit), column + 1), text: text))
            i = j
        }
        return clusters
    }
}

extension String {
    fileprivate mutating func trimTrailingSpaces() {
        while last == " " { removeLast() }
    }
}
