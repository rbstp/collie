import Testing

@testable import GhosttyTerminal

private let bg = TerminalRGB(0x28, 0x2C, 0x34)
private let fg = TerminalRGB(0xFF, 0xFF, 0xFF)

private func render(_ snapshot: String) throws -> TerminalFrame {
    let screen = try #require(TerminalScreen(background: bg, foreground: fg))
    return screen.render(ansiSnapshot: snapshot)
}

@Test func herdrStyledRows() throws {
    let frame = try render(
        "\u{1B}[0m\u{1B}[1m\u{1B}[38;2;153;153;153m4\u{1B}[0m plain\r\n  \u{1B}[38;2;118;159;240m\u{1B}[48;2;57;66;96m master \u{1B}[0m"
    )
    #expect(frame.rows == 2)
    #expect(frame.columns == 10)
    #expect(frame.background == bg)

    let bold = try #require(frame.runs.first)
    #expect(bold.row == 0 && bold.startColumn == 0 && bold.text == "4")
    #expect(bold.style.bold && bold.style.foreground == TerminalRGB(153, 153, 153))

    let plain = frame.runs[1]
    #expect(plain.text == " plain" && plain.style.foreground == fg && plain.style.background == nil)

    let badge = try #require(frame.runs.first { $0.text == " master " })
    #expect(badge.row == 1 && badge.startColumn == 2 && badge.endColumn == 10)
    #expect(badge.style.background == TerminalRGB(57, 66, 96))
}

@Test func wideAndBoxDrawingStayOnTheGrid() throws {
    let frame = try render("a\u{4E2D}b\u{2500}\u{2500}")
    let run = try #require(frame.runs.first)
    #expect(run.text == "a\u{4E2D}b\u{2500}\u{2500}")
    #expect(run.utf16Columns == [0, 1, 3, 4, 5])
    #expect(frame.columns == 6)
}

@Test func eachSnapshotReplacesThePrevious() throws {
    let screen = try #require(TerminalScreen(background: bg, foreground: fg))
    _ = screen.render(ansiSnapshot: "\u{1B}[7mfirst\r\nsecond\r\nthird")
    let frame = screen.render(ansiSnapshot: "next")
    #expect(frame.rows == 1)
    #expect(frame.runs.map(\.text) == ["next"])
    #expect(frame.runs[0].style.background == nil)
}

@Test func longRowsClipInsteadOfWrapping() throws {
    let long = String(repeating: "x", count: Int(TerminalScreen.maxColumns) + 50)
    let frame = try render("\(long)\r\nnext")
    #expect(frame.rows == 2)
    #expect(frame.columns == Int(TerminalScreen.maxColumns))
    #expect(frame.runs.last?.text == "next")
}

@Test func queriesAndControlSequencesAreHarmless() throws {
    let frame = try render("\u{1B}[6n\u{1B}[c\u{1B}]52;c;aGVsbG8=\u{07}\u{1B}_Ga=T,t=f;L2V0Yy9wYXNzd2Q=\u{1B}\\ok")
    #expect(frame.runs.map(\.text) == ["ok"])
}
