import GhosttyVt

public struct TerminalRGB: Hashable, Sendable {
    public var r: UInt8
    public var g: UInt8
    public var b: UInt8

    public init(_ r: UInt8, _ g: UInt8, _ b: UInt8) {
        self.r = r
        self.g = g
        self.b = b
    }

    init(_ c: GhosttyColorRgb) {
        self.init(c.r, c.g, c.b)
    }
}

public struct TerminalStyle: Hashable, Sendable {
    public var foreground: TerminalRGB
    public var background: TerminalRGB?
    public var bold = false
    public var italic = false
    public var faint = false
    public var underline = false
    public var strikethrough = false
    public var invisible = false
}

/// Consecutive cells of one row sharing a style. `utf16Columns[i]` is the column of `text.utf16[i]`.
public struct TerminalRun: Equatable, Sendable {
    public var row: Int
    public var startColumn: Int
    public var endColumn: Int
    public var text: String
    public var utf16Columns: [Int]
    public var style: TerminalStyle
}

public struct TerminalFrame: Equatable, Sendable {
    public var columns: Int
    public var rows: Int
    public var background: TerminalRGB
    public var foreground: TerminalRGB
    public var runs: [TerminalRun]
    /// Rows that continue the row above after a soft wrap.
    public var wrapContinuations: Set<Int> = []
    /// True when content scrolled while wrapping, so a full row may continue a soft wrap that
    /// `wrapContinuations` does not list.
    public var wrapsUnknown = false
}

/// A libghostty-vt terminal used as a snapshot renderer: no pty, no scrollback, no replies.
/// Nothing is registered for terminal output (WRITE_PTY), clipboard or other effects, so the
/// rendered content can never send bytes anywhere.
public final class TerminalScreen {
    // Bounds on untrusted snapshot dimensions. Without wrapping, wider rows are clipped.
    static let maxColumns: UInt16 = 500
    static let maxRows: UInt16 = 2000

    private let terminal: OpaquePointer
    private let renderState: OpaquePointer
    private let rowIterator: OpaquePointer
    private let rowCells: OpaquePointer
    private var columns = TerminalScreen.maxColumns
    private var rows: UInt16 = 1

    public init?(background: TerminalRGB, foreground: TerminalRGB) {
        var terminal: OpaquePointer?
        var renderState: OpaquePointer?
        var rowIterator: OpaquePointer?
        var rowCells: OpaquePointer?
        guard ghostty_terminal_new(nil, &terminal, Self.maxColumns, 1) == GHOSTTY_SUCCESS, let terminal else {
            return nil
        }
        guard ghostty_render_state_new(nil, &renderState) == GHOSTTY_SUCCESS, let renderState,
            ghostty_render_state_row_iterator_new(nil, &rowIterator) == GHOSTTY_SUCCESS, let rowIterator,
            ghostty_render_state_row_cells_new(nil, &rowCells) == GHOSTTY_SUCCESS, let rowCells
        else {
            ghostty_render_state_row_iterator_free(rowIterator)
            ghostty_render_state_free(renderState)
            ghostty_terminal_free(terminal)
            return nil
        }
        self.terminal = terminal
        self.renderState = renderState
        self.rowIterator = rowIterator
        self.rowCells = rowCells

        var bg = GhosttyColorRgb(r: background.r, g: background.g, b: background.b)
        var fg = GhosttyColorRgb(r: foreground.r, g: foreground.g, b: foreground.b)
        var noScrollback = 0
        ghostty_terminal_set(terminal, GHOSTTY_TERMINAL_OPT_COLOR_BACKGROUND, &bg)
        ghostty_terminal_set(terminal, GHOSTTY_TERMINAL_OPT_COLOR_FOREGROUND, &fg)
        ghostty_terminal_set(terminal, GHOSTTY_TERMINAL_OPT_SCROLLBACK_MAX_BYTES, &noScrollback)
    }

    deinit {
        ghostty_render_state_row_cells_free(rowCells)
        ghostty_render_state_row_iterator_free(rowIterator)
        ghostty_render_state_free(renderState)
        ghostty_terminal_free(terminal)
    }

    /// Replaces the screen with `snapshot`: rows of SGR-styled text joined by "\r\n". With
    /// `wrapColumns`, rows wrap at that width after trailing blanks are trimmed; without it they
    /// are clipped at `maxColumns`.
    public func render(ansiSnapshot snapshot: String, wrapColumns: Int? = nil) -> TerminalFrame {
        let wantedColumns: UInt16
        let wantedRows: UInt16
        var bytes: [UInt8]
        if let wrapColumns {
            wantedColumns = UInt16(min(max(wrapColumns, 1), Int(Self.maxColumns)))
            // Rows past the content are dropped by readFrame. preparedForWrapping keeps the
            // content within maxRows - 1 rows by an upper bound on each line's wrapped height.
            wantedRows = Self.maxRows
            bytes = Array("\u{1B}[?7h".utf8) + Array(Self.preparedForWrapping(snapshot, columns: Int(wantedColumns)).utf8)
        } else {
            bytes = Array(snapshot.utf8)
            let lines = bytes.reduce(1) { $1 == UInt8(ascii: "\n") ? $0 + 1 : $0 }
            wantedColumns = Self.maxColumns
            wantedRows = UInt16(min(lines, Int(Self.maxRows)))
            // Autowrap off so rows wider than the terminal clip instead of shifting every later row.
            bytes.insert(contentsOf: Array("\u{1B}[?7l".utf8), at: 0)
        }

        ghostty_terminal_reset(terminal)
        if wantedColumns != columns || wantedRows != rows {
            ghostty_terminal_resize(terminal, wantedColumns, wantedRows, 1, 1)
            columns = wantedColumns
            rows = wantedRows
        }
        var wrapContinuations: Set<Int>? = []
        if wrapColumns != nil {
            wrapContinuations = writeTrackingWraps(bytes)
        } else {
            write(bytes[...])
        }
        ghostty_render_state_update(renderState, terminal)
        var frame = readFrame()
        frame.wrapContinuations = wrapContinuations?.filter { $0 < frame.rows } ?? []
        frame.wrapsUnknown = wrapContinuations == nil
        return frame
    }

    /// Writes `bytes` a line at a time and returns the rows that continue a soft-wrapped line,
    /// from the cursor row before and after each line. ghostty_row_get is not in the render-state
    /// build of libghostty-vt. Once a line ends on the bottom row the screen may have scrolled, so
    /// row numbers are unknown and nil is returned.
    private func writeTrackingWraps(_ bytes: [UInt8]) -> Set<Int>? {
        var continuations: Set<Int> = []
        var reachedBottom = false
        var start = 0
        while start < bytes.count {
            let newline = bytes[start...].firstIndex(of: UInt8(ascii: "\n")) ?? bytes.count
            let top = cursorRow()
            write(bytes[start..<newline])
            let bottom = cursorRow()
            reachedBottom = reachedBottom || bottom >= Int(rows) - 1
            if bottom > top { continuations.formUnion(top + 1...bottom) }
            write(bytes[newline..<min(newline + 1, bytes.count)])
            start = newline + 1
        }
        return reachedBottom ? nil : continuations
    }

    /// Columns that fit `width` points of cells `cellWidth` wide, within 1...maxColumns.
    public static func wrapColumns(width: Double, cellWidth: Double) -> Int {
        guard cellWidth > 0, width.isFinite, width > 0 else { return 1 }
        return Int(min(max((width / cellWidth).rounded(.down), 1), Double(maxColumns)))
    }

    /// Drops spaces and tabs at the end of every row, also when SGR sequences sit between them,
    /// so padding to the Mac pane width does not wrap into blank rows. Those SGR sequences are
    /// kept so the style state for later rows is unchanged. Rows drawn only with box-drawing
    /// characters (rules, borders) get autowrap turned off around them, so they clip at the
    /// wrap width with their last character kept instead of wrapping into several rows. Other
    /// rows wider than `columns` that hold a run of one horizontal line character, like a rule
    /// with a label, lose the excess from their longest run when that leaves at least one of it.
    /// Widths follow Ghostty: East Asian wide and emoji presentation characters take two columns,
    /// marks and format characters none, and a tab advances to the next multiple of 8.
    /// The oldest rows are dropped, all but their SGR sequences, until the rest fits in
    /// `maxRows - 1` rows, so the screen never scrolls and soft wraps stay tracked.
    static func preparedForWrapping(_ snapshot: String, columns: Int) -> String {
        let scalars = Array(snapshot.unicodeScalars)
        var out: [Unicode.Scalar] = []
        out.reserveCapacity(scalars.count)
        var rowStarts: [Int] = []
        var heights: [Int] = []
        var rowStart = 0
        while rowStart <= scalars.count {
            let newline = scalars[rowStart...].firstIndex(of: "\n") ?? scalars.count
            var rowEnd = newline
            if rowEnd > rowStart && scalars[rowEnd - 1] == "\r" { rowEnd -= 1 }
            var contentEnd = rowStart
            var trailingSGR: [Range<Int>] = []
            var boxOnly = true
            var sawBox = false
            var width = 0
            var contentWidth = 0
            var run = 0..<0
            var runColumn = 0
            var longestRun = 0..<0
            var longestRunColumn = 0
            var i = rowStart
            while i < rowEnd {
                if let end = Self.sgrEnd(scalars, at: i, before: rowEnd) {
                    trailingSGR.append(i..<end)
                    i = end
                    continue
                }
                if Self.horizontalLines.contains(scalars[i].value) {
                    if run.upperBound == i && scalars[run.lowerBound] == scalars[i] {
                        run = run.lowerBound..<(i + 1)
                    } else {
                        run = i..<(i + 1)
                        runColumn = width
                    }
                    if run.count > longestRun.count { (longestRun, longestRunColumn) = (run, runColumn) }
                }
                width += Self.cellWidth(scalars[i], column: width)
                if scalars[i] != " " && scalars[i] != "\t" {
                    contentEnd = i + 1
                    contentWidth = width
                    trailingSGR.removeAll()
                    if (0x2500...0x257F).contains(scalars[i].value) {
                        sawBox = true
                    } else {
                        boxOnly = false
                    }
                }
                i += 1
            }
            let clip = boxOnly && sawBox
            var cut = 0
            if !clip && contentWidth > columns {
                // Tabs after the run move to other stops once it shrinks, so remeasure the tail.
                var tried = contentWidth - columns
                while tried < longestRun.count {
                    let tail = longestRun.upperBound..<contentEnd
                    if Self.width(of: scalars, in: tail, from: longestRunColumn + longestRun.count - tried) <= columns {
                        cut = tried
                        break
                    }
                    tried += 1
                }
            }
            rowStarts.append(out.count)
            // A wide character that does not fit wraps early, leaving at most one column unused.
            let shown = contentWidth - cut
            let perRow = max(columns - 1, 1)
            heights.append(clip || shown <= columns ? 1 : (shown + perRow - 1) / perRow)
            if clip { out.append(contentsOf: "\u{1B}[?7l".unicodeScalars) }
            if cut > 0 {
                out.append(contentsOf: scalars[rowStart..<(longestRun.upperBound - cut)])
                out.append(contentsOf: scalars[longestRun.upperBound..<contentEnd])
            } else {
                out.append(contentsOf: scalars[rowStart..<contentEnd])
            }
            if clip { out.append(contentsOf: "\u{1B}[?7h".unicodeScalars) }
            for range in trailingSGR { out.append(contentsOf: scalars[range]) }
            out.append(contentsOf: scalars[rowEnd..<min(newline + 1, scalars.count)])
            rowStart = newline + 1
        }
        var first = heights.count
        var height = 0
        while first > 0 && (first == heights.count || height + heights[first - 1] <= Int(maxRows) - 1) {
            first -= 1
            height += heights[first]
        }
        guard first > 0 else { return String(String.UnicodeScalarView(out)) }
        let dropped = rowStarts[first]
        var kept: [Unicode.Scalar] = []
        var i = 0
        while i < dropped {
            if let end = sgrEnd(out, at: i, before: dropped) {
                kept.append(contentsOf: out[i..<end])
                i = end
            } else {
                i += 1
            }
        }
        kept.append(contentsOf: out[dropped...])
        return String(String.UnicodeScalarView(kept))
    }

    static let horizontalLines: Set<UInt32> = [0x2500, 0x2501, 0x2504, 0x2505, 0x2508, 0x2509, 0x254C, 0x254D, 0x2550]

    private static func sgrEnd(_ scalars: [Unicode.Scalar], at i: Int, before end: Int) -> Int? {
        guard scalars[i] == "\u{1B}", i + 1 < end, scalars[i + 1] == "[" else { return nil }
        var j = i + 2
        while j < end, "0123456789;:".unicodeScalars.contains(scalars[j]) { j += 1 }
        return j < end && scalars[j] == "m" ? j + 1 : nil
    }

    private static func width(of scalars: [Unicode.Scalar], in range: Range<Int>, from column: Int) -> Int {
        var width = column
        var i = range.lowerBound
        while i < range.upperBound {
            if let end = sgrEnd(scalars, at: i, before: range.upperBound) {
                i = end
                continue
            }
            width += cellWidth(scalars[i], column: width)
            i += 1
        }
        return width
    }

    static func cellWidth(_ scalar: Unicode.Scalar, column: Int) -> Int {
        if scalar == "\t" { return 8 - column % 8 }
        if scalar.value < 0x100 { return 1 }
        switch scalar.properties.generalCategory {
        case .nonspacingMark, .enclosingMark, .format: return 0
        default: break
        }
        if (0x1160...0x11FF).contains(scalar.value) { return 0 }
        if scalar.properties.isEmojiPresentation || wideRanges.contains(where: { $0.contains(scalar.value) }) { return 2 }
        return 1
    }

    private static let wideRanges: [ClosedRange<UInt32>] = [
        0x1100...0x115F, 0x231A...0x231B, 0x2329...0x232A, 0x2E80...0x303E, 0x3041...0x33FF,
        0x3400...0x4DBF, 0x4E00...0x9FFF, 0xA000...0xA4CF, 0xA960...0xA97F, 0xAC00...0xD7A3,
        0xF900...0xFAFF, 0xFE10...0xFE19, 0xFE30...0xFE6F, 0xFF00...0xFF60, 0xFFE0...0xFFE6,
        0x16FE0...0x16FE4, 0x17000...0x18CFF, 0x1B000...0x1B2FF, 0x1F200...0x1F2FF,
        0x20000...0x2FFFD, 0x30000...0x3FFFD,
    ]

    private func write(_ bytes: ArraySlice<UInt8>) {
        bytes.withUnsafeBufferPointer { ghostty_terminal_vt_write(terminal, $0.baseAddress, $0.count) }
    }

    private func cursorRow() -> Int {
        var row: UInt16 = 0
        ghostty_terminal_get(terminal, GHOSTTY_TERMINAL_DATA_CURSOR_Y, &row)
        return Int(row)
    }

    private func readFrame() -> TerminalFrame {
        var colors = GhosttyRenderStateColors()
        colors.size = MemoryLayout<GhosttyRenderStateColors>.size
        ghostty_render_state_get(renderState, GHOSTTY_RENDER_STATE_DATA_COLORS, &colors)
        let defaultBackground = TerminalRGB(colors.background)
        let defaultForeground = TerminalRGB(colors.foreground)

        var iterator: OpaquePointer? = rowIterator
        ghostty_render_state_get(renderState, GHOSTTY_RENDER_STATE_DATA_ROW_ITERATOR, &iterator)
        var cells: OpaquePointer? = rowCells

        var runs: [TerminalRun] = []
        var usedColumns = 0
        var usedRows = 0
        var row = 0
        var codepoints = [UInt32](repeating: 0, count: 16)
        while ghostty_render_state_row_iterator_next(rowIterator) {
            ghostty_render_state_row_get(rowIterator, GHOSTTY_RENDER_STATE_ROW_DATA_CELLS, &cells)
            var current: TerminalRun?
            var column = 0
            while ghostty_render_state_row_cells_next(rowCells) {
                defer { column += 1 }
                var length: UInt32 = 0
                ghostty_render_state_row_cells_get(rowCells, GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_GRAPHEMES_LEN, &length)
                let style = cellStyle(defaultBackground: defaultBackground, defaultForeground: defaultForeground)
                if length == 0 && style.background == nil {
                    // A wide character's spacer cell keeps its style: extend so decorations cover both cells.
                    if current?.style == style && current?.endColumn == column {
                        current!.endColumn = column + 1
                        continue
                    }
                    if let run = current { runs.append(run) }
                    current = nil
                    continue
                }
                var text = ""
                if length > 0 {
                    if Int(length) > codepoints.count {
                        codepoints = [UInt32](repeating: 0, count: Int(length))
                    }
                    ghostty_render_state_row_cells_get(rowCells, GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_GRAPHEMES_BUF, &codepoints)
                    for value in codepoints.prefix(Int(length)) {
                        text.unicodeScalars.append(Unicode.Scalar(value) ?? "\u{FFFD}")
                    }
                }
                if current?.style != style || current?.endColumn != column {
                    if let run = current { runs.append(run) }
                    current = TerminalRun(row: row, startColumn: column, endColumn: column, text: "", utf16Columns: [], style: style)
                }
                current!.text += text
                current!.utf16Columns += repeatElement(column, count: text.utf16.count)
                current!.endColumn = column + 1
                if length > 0 || style.background != defaultBackground {
                    usedColumns = max(usedColumns, column + 1)
                    usedRows = row + 1
                }
            }
            if let run = current { runs.append(run) }
            row += 1
        }
        ghostty_render_state_clean(renderState)
        return TerminalFrame(
            columns: usedColumns,
            rows: usedRows,
            background: defaultBackground,
            foreground: defaultForeground,
            runs: runs.filter { $0.row < usedRows && $0.startColumn < usedColumns }
        )
    }

    private func cellStyle(defaultBackground: TerminalRGB, defaultForeground: TerminalRGB) -> TerminalStyle {
        var fg = GhosttyColorRgb()
        var bg = GhosttyColorRgb()
        var foreground = defaultForeground
        var background: TerminalRGB?
        if ghostty_render_state_row_cells_get(rowCells, GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_FG_COLOR, &fg) == GHOSTTY_SUCCESS {
            foreground = TerminalRGB(fg)
        }
        if ghostty_render_state_row_cells_get(rowCells, GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_BG_COLOR, &bg) == GHOSTTY_SUCCESS {
            background = TerminalRGB(bg)
        }
        var styled = false
        ghostty_render_state_row_cells_get(rowCells, GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_HAS_STYLING, &styled)
        guard styled else { return TerminalStyle(foreground: foreground, background: background) }

        var raw = GhosttyStyle()
        raw.size = MemoryLayout<GhosttyStyle>.size
        ghostty_render_state_row_cells_get(rowCells, GHOSTTY_RENDER_STATE_ROW_CELLS_DATA_STYLE, &raw)
        if raw.inverse {
            (foreground, background) = (background ?? defaultBackground, foreground)
        }
        return TerminalStyle(
            foreground: foreground,
            background: background,
            bold: raw.bold,
            italic: raw.italic,
            faint: raw.faint,
            underline: raw.underline != Int32(GHOSTTY_SGR_UNDERLINE_NONE.rawValue),
            strikethrough: raw.strikethrough,
            invisible: raw.invisible
        )
    }
}
