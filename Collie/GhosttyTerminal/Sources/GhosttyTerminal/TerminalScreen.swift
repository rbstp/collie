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
}

/// A libghostty-vt terminal used as a snapshot renderer: no pty, no scrollback, no replies.
/// Nothing is registered for terminal output (WRITE_PTY), clipboard or other effects, so the
/// rendered content can never send bytes anywhere.
public final class TerminalScreen {
    // Bounds on untrusted snapshot dimensions. Without wrapping, wider rows are clipped.
    static let maxColumns: UInt16 = 500
    static let maxRows: UInt16 = 500

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
            // Wrapped height is unknown before writing; rows past the content are dropped by
            // readFrame, and content beyond maxRows scrolls the oldest rows off the top.
            wantedRows = Self.maxRows
            bytes = Array("\u{1B}[?7h".utf8) + Array(Self.trimmingTrailingBlanks(snapshot).utf8)
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
        bytes.withUnsafeBufferPointer { ghostty_terminal_vt_write(terminal, $0.baseAddress, $0.count) }
        ghostty_render_state_update(renderState, terminal)
        return readFrame()
    }

    /// Columns that fit `width` points of cells `cellWidth` wide, within 1...maxColumns.
    public static func wrapColumns(width: Double, cellWidth: Double) -> Int {
        guard cellWidth > 0, width.isFinite, width > 0 else { return 1 }
        return Int(min(max((width / cellWidth).rounded(.down), 1), Double(maxColumns)))
    }

    /// Drops spaces and tabs at the end of every row, also when SGR sequences sit between them,
    /// so padding to the Mac pane width does not wrap into blank rows. Those SGR sequences are
    /// kept so the style state for later rows is unchanged.
    static func trimmingTrailingBlanks(_ snapshot: String) -> String {
        let scalars = Array(snapshot.unicodeScalars)
        var out = String.UnicodeScalarView()
        out.reserveCapacity(scalars.count)
        var rowStart = 0
        while rowStart <= scalars.count {
            let newline = scalars[rowStart...].firstIndex(of: "\n") ?? scalars.count
            var rowEnd = newline
            if rowEnd > rowStart && scalars[rowEnd - 1] == "\r" { rowEnd -= 1 }
            var contentEnd = rowStart
            var trailingSGR: [Range<Int>] = []
            var i = rowStart
            while i < rowEnd {
                if scalars[i] == "\u{1B}", i + 1 < rowEnd, scalars[i + 1] == "[" {
                    var j = i + 2
                    while j < rowEnd, "0123456789;:".unicodeScalars.contains(scalars[j]) { j += 1 }
                    if j < rowEnd, scalars[j] == "m" {
                        trailingSGR.append(i..<(j + 1))
                        i = j + 1
                        continue
                    }
                }
                if scalars[i] != " " && scalars[i] != "\t" {
                    contentEnd = i + 1
                    trailingSGR.removeAll()
                }
                i += 1
            }
            out.append(contentsOf: scalars[rowStart..<contentEnd])
            for range in trailingSGR { out.append(contentsOf: scalars[range]) }
            out.append(contentsOf: scalars[rowEnd..<min(newline + 1, scalars.count)])
            rowStart = newline + 1
        }
        return String(out)
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
