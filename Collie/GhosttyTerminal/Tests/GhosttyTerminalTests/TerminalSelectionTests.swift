import Testing

@testable import GhosttyTerminal

private func render(_ snapshot: String, wrapColumns: Int? = nil) throws -> TerminalFrame {
    let screen = try #require(TerminalScreen(background: TerminalRGB(0, 0, 0), foreground: TerminalRGB(255, 255, 255)))
    return screen.render(ansiSnapshot: snapshot, wrapColumns: wrapColumns)
}

private func cell(_ row: Int, _ column: Int) -> TerminalCell {
    TerminalCell(row: row, column: column)
}

private func selection(_ a: TerminalCell, _ b: TerminalCell) -> TerminalSelection {
    TerminalSelection(a, b)
}

@Test func wordUnderAPoint() throws {
    let frame = try render("$ cat ~/src/main.rs -- \u{2502}x\u{2500}\u{2500} \u{1B}[1mbold\u{1B}[0mplain")
    #expect(TerminalSelection.word(at: cell(0, 8), in: frame) == selection(cell(0, 6), cell(0, 18)))
    #expect(TerminalSelection.word(at: cell(0, 20), in: frame) == selection(cell(0, 20), cell(0, 20)))
    #expect(TerminalSelection.word(at: cell(0, 0), in: frame) == selection(cell(0, 0), cell(0, 0)))
    #expect(TerminalSelection.word(at: cell(0, 5), in: frame) == selection(cell(0, 5), cell(0, 5)))
    #expect(TerminalSelection.word(at: cell(0, 23), in: frame) == selection(cell(0, 23), cell(0, 23)))
    #expect(TerminalSelection.word(at: cell(0, 24), in: frame) == selection(cell(0, 24), cell(0, 24)))
    #expect(TerminalSelection.word(at: cell(0, 25), in: frame) == selection(cell(0, 25), cell(0, 25)))
    #expect(TerminalSelection.word(at: cell(0, 29), in: frame) == selection(cell(0, 28), cell(0, 36)))
    #expect(TerminalSelection.word(at: cell(0, 60), in: frame) == selection(cell(0, 60), cell(0, 60)))
}

@Test func wordCoversBothCellsOfAWideCharacter() throws {
    let frame = try render("x \u{4E2D}\u{6587} y")
    #expect(TerminalSelection.word(at: cell(0, 3), in: frame) == selection(cell(0, 2), cell(0, 5)))
    #expect(selection(cell(0, 3), cell(0, 4)).text(in: frame) == "\u{4E2D}\u{6587}")
}

@Test func endsAreOrderedWhicheverWayTheyAreGiven() {
    let reversed = selection(cell(2, 5), cell(0, 3))
    #expect(reversed.start == cell(0, 3) && reversed.end == cell(2, 5))
    #expect(selection(cell(1, 9), cell(1, 2)) == selection(cell(1, 2), cell(1, 9)))
}

@Test func dragFromAWordKeepsTheWord() {
    let word = selection(cell(1, 4), cell(1, 8))
    #expect(word.extended(to: cell(1, 6)) == word)
    #expect(word.extended(to: cell(3, 0)) == selection(cell(1, 4), cell(3, 0)))
    #expect(word.extended(to: cell(0, 10)) == selection(cell(0, 10), cell(1, 8)))
}

@Test func handlesSwapWhenTheyCross() {
    let selected = selection(cell(0, 2), cell(0, 6))
    let pastEnd = selected.moving(.start, to: cell(1, 1))
    #expect(pastEnd.selection == selection(cell(0, 6), cell(1, 1)) && pastEnd.edge == .end)
    let beforeStart = selected.moving(.end, to: cell(0, 0))
    #expect(beforeStart.selection == selection(cell(0, 0), cell(0, 2)) && beforeStart.edge == .start)
    let onto = selected.moving(.end, to: cell(0, 2))
    #expect(onto.selection == selection(cell(0, 2), cell(0, 2)) && onto.edge == .end)
    let inside = selected.moving(.start, to: cell(0, 4))
    #expect(inside.selection == selection(cell(0, 4), cell(0, 6)) && inside.edge == .start)
}

@Test func singleCellText() throws {
    let frame = try render("abc")
    #expect(selection(cell(0, 1), cell(0, 1)).text(in: frame) == "b")
    #expect(selection(cell(0, 0), cell(0, 0)).text(in: try render(" x")) == "")
}

@Test func multiRowTextFollowsReadingOrder() throws {
    let frame = try render("one two\r\n  three \u{1B}[31mfour\u{1B}[0m\r\nfive")
    #expect(selection(cell(0, 4), cell(2, 1)).text(in: frame) == "two\n  three four\nfi")
    #expect(selection(cell(1, 4), cell(1, 9)).text(in: frame) == "ree fo")
}

@Test func softWrappedRowsJoinWithoutANewline() throws {
    let frame = try render("hello world again\r\nabcdef\r\nnext", wrapColumns: 6)
    #expect(frame.rows == 5)
    #expect(frame.wrapContinuations == [1, 2])
    #expect(selection(cell(0, 0), cell(4, 5)).text(in: frame) == "hello world again\nabcdef\nnext")
    #expect(selection(cell(1, 3), cell(2, 1)).text(in: frame) == "ld ag")
    #expect(try render("hello world again").wrapContinuations.isEmpty)
}

@Test func aThousandLineHistoryKeepsEveryWrap() throws {
    let lines = (0..<1000).map { "\($0) " + String(repeating: "x", count: $0 % 4 == 0 ? 150 : 60) }
    let frame = try render(lines.joined(separator: "\r\n"), wrapColumns: 98)
    #expect(frame.rows == 1250)
    #expect(!frame.wrapsUnknown)
    #expect(frame.wrapContinuations.count == 250)
    #expect(selection(cell(0, 0), cell(1, 53)).text(in: frame) == lines[0])
}

@Test func wrapRowsAreUnknownOnceTheScreenScrolls() throws {
    let lines = Array(repeating: String(repeating: "x", count: 15), count: 1100).joined(separator: "\r\n")
    let frame = try render(lines, wrapColumns: 10)
    #expect(frame.rows == Int(TerminalScreen.maxRows))
    #expect(frame.wrapContinuations.isEmpty)
    #expect(frame.wrapsUnknown)
    #expect(try !render("hello world again", wrapColumns: 6).wrapsUnknown)
}

@Test func wideCharactersAreCopiedOnce() throws {
    let frame = try render("a\u{4E2D}b")
    #expect(selection(cell(0, 2), cell(0, 2)).text(in: frame) == "\u{4E2D}")
    #expect(selection(cell(0, 1), cell(0, 1)).text(in: frame) == "\u{4E2D}")
    #expect(selection(cell(0, 2), cell(0, 3)).text(in: frame) == "\u{4E2D}b")
    #expect(selection(cell(0, 0), cell(0, 3)).text(in: frame) == "a\u{4E2D}b")
}

@Test func trailingSpacesAreTrimmedAtLineEnds() throws {
    let frame = try render("ab   \r\n    \r\n  cd  e\r\n\u{1B}[44mbg  \u{1B}[0m")
    #expect(selection(cell(0, 0), cell(3, 9)).text(in: frame) == "ab\n\n  cd  e\nbg")
    #expect(selection(cell(2, 0), cell(2, 3)).text(in: frame) == "  cd")
}

@Test func selectionBeyondTheContentIsClamped() throws {
    let frame = try render("abc\r\ndefgh")
    #expect(frame.rows == 2 && frame.columns == 5)
    #expect(TerminalCell.at(x: -40, y: -40, cellWidth: 7, cellHeight: 14, inset: 8, in: frame) == cell(0, 0))
    #expect(TerminalCell.at(x: 900, y: 900, cellWidth: 7, cellHeight: 14, inset: 8, in: frame) == cell(1, 4))
    #expect(TerminalCell.at(x: 900, y: 10, cellWidth: 7, cellHeight: 14, inset: 8, in: frame) == cell(0, 4))
    #expect(TerminalCell.at(x: 1, y: 23, cellWidth: 7, cellHeight: 14, inset: 8, in: frame) == cell(1, 0))
    #expect(selection(cell(0, 1), cell(1, 99)).text(in: frame) == "bc\ndefgh")
    #expect(selection(cell(5, 0), cell(9, 9)).text(in: frame) == "")
    #expect(selection(cell(0, 1), cell(1, 99)).columns(inRow: 1, columns: frame.columns) == 0...4)
    #expect(selection(cell(0, 9), cell(0, 12)).columns(inRow: 0, columns: frame.columns) == nil)
}

@Test func newFramesKeepTheSelectionWhileItsRowsExist() throws {
    let selected = selection(cell(0, 2), cell(1, 7))
    #expect(selected.clamped(to: try render("abc\r\ndefghijklm")) == selected)
    #expect(selected.clamped(to: try render("abc\r\ndef")) == selection(cell(0, 2), cell(1, 2)))
    #expect(selected.clamped(to: try render("abcdefghij")) == nil)
    #expect(selected.clamped(to: try render("")) == nil)
}

@Test func pointsMapToCellsThroughInsetAndContentOffset() {
    let frame = TerminalFrame(
        columns: 80, rows: 40, background: TerminalRGB(0, 0, 0), foreground: TerminalRGB(255, 255, 255), runs: []
    )
    let (width, height, inset) = (6.5, 14.0, 8.0)
    let contentOffset = (x: 13.0, y: 140.0)
    func at(visibleX: Double, visibleY: Double) -> TerminalCell? {
        TerminalCell.at(
            x: visibleX + contentOffset.x, y: visibleY + contentOffset.y,
            cellWidth: width, cellHeight: height, inset: inset, in: frame
        )
    }
    #expect(at(visibleX: 20, visibleY: 30) == cell(11, 3))
    #expect(at(visibleX: -5, visibleY: -132) == cell(0, 0))
    #expect(at(visibleX: 8 - 13 + 6.5 * 4, visibleY: 8 - 140 + 14 * 12) == cell(12, 4))
    #expect(at(visibleX: 8 - 13 + 6.5 * 4 - 0.01, visibleY: 8 - 140 + 14 * 12 - 0.01) == cell(11, 3))
    #expect(TerminalCell.at(x: 10, y: 10, cellWidth: 0, cellHeight: 14, inset: 8, in: frame) == nil)
}
