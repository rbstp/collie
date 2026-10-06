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

@Test func longRowsWrapAtTheGivenWidth() throws {
    let screen = try #require(TerminalScreen(background: bg, foreground: fg))
    for columns in [4, 37, 50, 57, 200] {
        let frame = screen.render(ansiSnapshot: "\u{1B}[1m\(String(repeating: "x", count: 200))\u{1B}[0m\r\nnext", wrapColumns: columns)
        let wrapped = (200 + columns - 1) / columns
        #expect(frame.rows == wrapped + 1)
        #expect(frame.columns == columns)
        #expect(frame.runs.filter(\.style.bold).map(\.text).joined() == String(repeating: "x", count: 200))
        #expect(frame.runs.last?.row == wrapped && frame.runs.last?.text == "next")
    }
}

@Test func wrappingTrimsPaddingBeforeWrapping() throws {
    let screen = try #require(TerminalScreen(background: bg, foreground: fg))
    let padded = "ab\u{1B}[0m" + String(repeating: " ", count: 150) + "\u{1B}[48;5;1m" + String(repeating: " ", count: 40) + "\u{1B}[0m"
    let frame = screen.render(ansiSnapshot: "\(padded)\r\ncd", wrapColumns: 40)
    #expect(frame.rows == 2)
    #expect(frame.runs.map(\.text) == ["ab", "cd"])
    #expect(screen.render(ansiSnapshot: "\(padded)\r\ncd").columns == 192)
}

@Test func boxDrawingRowsClipAtTheWrapWidth() throws {
    let screen = try #require(TerminalScreen(background: bg, foreground: fg))
    let border = "\u{256D}" + String(repeating: "\u{2500}", count: 120) + "\u{256E}"
    let rule = "\u{1B}[38;5;8m" + String(repeating: "\u{2500}", count: 120) + "\u{1B}[0m"
    let text = "\u{2502} " + String(repeating: "z", count: 60)
    let frame = screen.render(ansiSnapshot: "\(border)\r\n\(rule)\r\n\(text)\r\nend", wrapColumns: 40)
    #expect(frame.rows == 5)
    #expect(frame.columns == 40)
    let rows = Dictionary(grouping: frame.runs, by: \.row).mapValues { $0.map(\.text).joined() }
    #expect(rows[0] == "\u{256D}" + String(repeating: "\u{2500}", count: 38) + "\u{256E}")
    #expect(rows[1] == String(repeating: "\u{2500}", count: 40))
    #expect(rows[4] == "end")
}

@Test func wrapModeCanBeTurnedOffAgain() throws {
    let screen = try #require(TerminalScreen(background: bg, foreground: fg))
    let long = String(repeating: "y", count: 200)
    #expect(screen.render(ansiSnapshot: long, wrapColumns: 50).rows == 4)
    let frame = screen.render(ansiSnapshot: "\(long)\r\nnext")
    #expect(frame.rows == 2)
    #expect(frame.columns == 200)
    #expect(frame.runs.first?.text == long)
}

@Test func trailingBlanksAndPaddingAreTrimmedPerRow() {
    let cases = [
        ("", ""),
        ("abc   ", "abc"),
        ("abc \t \r\n  x  \r\n", "abc\r\n  x\r\n"),
        ("a\u{1B}[0m   \u{1B}[48;2;1;2;3m  \u{1B}[0m\r\nb", "a\u{1B}[0m\u{1B}[48;2;1;2;3m\u{1B}[0m\r\nb"),
        ("\u{1B}[1m a \u{1B}[22m b\u{1B}[0m ", "\u{1B}[1m a \u{1B}[22m b\u{1B}[0m"),
        ("   \u{1B}[0m\n\n x", "\u{1B}[0m\n\n x"),
        ("\u{4E2D}\u{00A0}", "\u{4E2D}\u{00A0}"),
        ("\u{1B}[2m\u{2500}\u{2500} \u{1B}[0m  \r\nx", "\u{1B}[?7l\u{1B}[2m\u{2500}\u{2500}\u{1B}[?7h\u{1B}[0m\r\nx"),
        ("\u{2502} a \u{2502}", "\u{2502} a \u{2502}"),
        ("\u{2580}\u{2580}", "\u{2580}\u{2580}"),
    ]
    for (input, trimmed) in cases {
        #expect(TerminalScreen.preparedForWrapping(input, columns: Int(TerminalScreen.maxColumns)) == trimmed, "\(input.debugDescription)")
    }
}

@Test func labeledRulesShrinkToTheWrapWidth() throws {
    let screen = try #require(TerminalScreen(background: bg, foreground: fg))
    let purple = TerminalRGB(177, 185, 249)
    let rule = "\u{1B}[0m\u{1B}[38;2;136;136;136m" + String(repeating: "\u{2500}", count: 178)
        + " \u{1B}[38;2;177;185;249multracode\u{1B}[0m \u{2500}"
    let frame = screen.render(ansiSnapshot: "\(rule)\r\nnext", wrapColumns: 50)
    #expect(frame.rows == 2)
    #expect(frame.columns == 50)
    let rows = Dictionary(grouping: frame.runs, by: \.row).mapValues { $0.map(\.text).joined() }
    #expect(rows[0] == String(repeating: "\u{2500}", count: 38) + " ultracode \u{2500}")
    #expect(rows[1] == "next")
    let label = try #require(frame.runs.first { $0.text == "ultracode" })
    #expect(label.style.foreground == purple && label.startColumn == 39)
}

@Test func labeledRuleShrinkingKeepsTextAndEscapes() {
    let line = { (count: Int) in String(repeating: "\u{2500}", count: count) }
    let cases = [
        (
            "\u{256D}" + line(3) + " \u{1B}[1mTitle\u{1B}[22m " + line(100) + "\u{256E}",
            "\u{256D}" + line(3) + " \u{1B}[1mTitle\u{1B}[22m " + line(28) + "\u{256E}"
        ),
        (line(30) + " fits " + line(4), line(30) + " fits " + line(4)),
        (line(25) + " " + String(repeating: "x", count: 39), line(25) + " \u{1B}E" + String(repeating: "x", count: 39)),
        (line(26) + " " + String(repeating: "x", count: 38) + "   \r\nb", line(1) + " " + String(repeating: "x", count: 38) + "\r\nb"),
        (line(100) + " \u{26A1} ultracode " + line(1), line(25) + " \u{26A1} ultracode " + line(1)),
        (line(100) + " \u{65E5}\u{672C} " + line(1), line(33) + " \u{65E5}\u{672C} " + line(1)),
        (line(100) + " e\u{301}\u{FE0F} x " + line(1), line(34) + " e\u{301}\u{FE0F} x " + line(1)),
        ("a\t" + line(100) + "b", "a\t" + line(31) + "b"),
        (line(100) + "\tb", line(31) + "\tb"),
    ]
    for (input, prepared) in cases {
        #expect(TerminalScreen.preparedForWrapping(input, columns: 40) == prepared, "\(input.debugDescription)")
    }
}

@Test func longLinesBreakAtWords() {
    let cases = [
        ("the quick brown fox jumps", 10, "the quick \u{1B}Ebrown fox \u{1B}Ejumps"),
        ("ab abcdefghijklmnop", 8, "ab \u{1B}Eabcdefgh\u{1B}Eijklmnop"),
        ("abcde fg", 5, "abcde\u{1B}E fg"),
        ("ab \u{4E2D}\u{6587}\u{5B57}", 6, "ab \u{1B}E\u{4E2D}\u{6587}\u{5B57}"),
        ("\u{4E2D}\u{6587}\u{5B57}\u{4E2D}", 7, "\u{4E2D}\u{6587}\u{5B57}\u{1B}E\u{4E2D}"),
        ("\u{1B}[1mhello \u{1B}[31mworld\u{1B}[0m", 8, "\u{1B}[1mhello \u{1B}E\u{1B}[31mworld\u{1B}[0m"),
        ("  - one two three four", 12, "  - one two \u{1B}E\u{1B}[4Cthree \u{1B}E\u{1B}[4Cfour"),
        ("\u{23FA} said hello there", 13, "\u{23FA} said hello \u{1B}E\u{1B}[2Cthere"),
        ("        deep in it", 14, "        deep \u{1B}Ein it"),
        ("a\tbcdefgh ij", 12, "a\t\u{1B}Ebcdefgh ij"),
    ]
    for (input, columns, prepared) in cases {
        #expect(TerminalScreen.preparedForWrapping(input, columns: columns) == prepared, "\(input.debugDescription)")
    }
}

@Test func boxPaddingShrinksToFitTheWrapWidth() {
    let pad = { (count: Int) in String(repeating: " ", count: count) }
    let cases = [
        (
            "\u{2502} Phases" + pad(40) + "\u{2502} agent" + pad(30) + "7m45s \u{2502}",
            "\u{2502} Phases  \u{2502} agent  7m45s \u{2502}"
        ),
        (
            "  \u{2502} \u{1B}[1mPhases\u{1B}[0m" + pad(50) + "\u{1B}[2m\u{2502}\u{1B}[0m",
            "  \u{2502} \u{1B}[1mPhases\u{1B}[0m  \u{1B}[2m\u{2502}\u{1B}[0m"
        ),
        ("\u{2502} a" + pad(30) + "\u{2502}", "\u{2502} a" + pad(30) + "\u{2502}"),
        ("Phases" + pad(40) + "agent", "Phases" + pad(34) + "\u{1B}E" + pad(6) + "agent"),
    ]
    for (input, prepared) in cases {
        #expect(TerminalScreen.preparedForWrapping(input, columns: 40) == prepared, "\(input.debugDescription)")
    }
}

@Test func paddedPanelsKeepOneRowPerLine() throws {
    let screen = try #require(TerminalScreen(background: bg, foreground: fg))
    let line = { (count: Int) in String(repeating: "\u{2500}", count: count) }
    let pad = { (count: Int) in String(repeating: " ", count: count) }
    let row = "\u{2502} \u{2713} plan" + pad(60) + "7m45s \u{2502} reviewer" + pad(90) + "\u{2502}"
    let panel = [
        "\u{256D}" + line(20) + " Workflow " + line(150) + "\u{256E}", row, "\u{2570}" + line(180) + "\u{256F}",
    ]
    let frame = screen.render(ansiSnapshot: panel.joined(separator: "\r\n"), wrapColumns: 40)
    #expect(frame.rows == 3)
    let rows = Dictionary(grouping: frame.runs, by: \.row).mapValues { $0.map(\.text).joined() }
    #expect(rows[1] == "\u{2502} \u{2713} plan  7m45s \u{2502} reviewer  \u{2502}")
    #expect(screen.render(ansiSnapshot: row).runs.map(\.text) == [row])
}

@Test func cellWidthsMatchGhostty() throws {
    let screen = try #require(TerminalScreen(background: bg, foreground: fg))
    let samples: [Unicode.Scalar] = [
        "a", "\u{E9}", "\u{2500}", "\u{2764}", "\u{301}", "\u{FE0F}", "\u{200D}", "\u{26A1}", "\u{65E5}",
        "\u{AC00}", "\u{FF21}", "\u{1F600}", "\u{1F680}", "\u{3000}",
    ]
    for scalar in samples {
        let frame = screen.render(ansiSnapshot: "x\(scalar)|")
        let run = try #require(frame.runs.first { $0.text.contains("|") })
        let index = try #require(run.text.utf16.firstIndex(of: UInt16(UInt8(ascii: "|"))))
        let column = run.utf16Columns[run.text.utf16.distance(from: run.text.utf16.startIndex, to: index)]
        #expect(column == 1 + TerminalScreen.cellWidth(scalar, column: 1), "\(scalar.escaped(asASCII: true))")
    }
    #expect(TerminalScreen.cellWidth("\t", column: 0) == 8)
    #expect(TerminalScreen.cellWidth("\t", column: 13) == 3)
}

@Test func wrapColumnsFitTheWidth() {
    #expect(TerminalScreen.wrapColumns(width: 377, cellWidth: 6.6) == 57)
    #expect(TerminalScreen.wrapColumns(width: 66, cellWidth: 6.6) == 10)
    #expect(TerminalScreen.wrapColumns(width: 65.9, cellWidth: 6.6) == 9)
    #expect(TerminalScreen.wrapColumns(width: 3, cellWidth: 6.6) == 1)
    #expect(TerminalScreen.wrapColumns(width: 100_000, cellWidth: 6.6) == Int(TerminalScreen.maxColumns))
    #expect(TerminalScreen.wrapColumns(width: 377, cellWidth: 0) == 1)
}
