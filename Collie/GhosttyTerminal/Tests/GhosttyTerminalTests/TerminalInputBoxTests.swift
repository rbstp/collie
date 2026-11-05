import Testing

@testable import GhosttyTerminal

private func render(_ snapshot: String, wrapColumns: Int? = nil) throws -> TerminalFrame {
    let screen = try #require(TerminalScreen(background: TerminalRGB(0, 0, 0), foreground: TerminalRGB(255, 255, 255)))
    return screen.render(ansiSnapshot: snapshot, wrapColumns: wrapColumns)
}

private let rule = String(repeating: "\u{2500}", count: 60)

/// Claude Code's screen around its input box, rows padded to the pane width as herdr reads them.
private func claude(_ box: [String], below: [String] = ["  Opus 5.5 high", "  \u{23F5}\u{23F5} auto mode on"]) -> String {
    (["\u{23FA} Done.", ""] + [rule] + box + [rule] + below)
        .map { $0 + String(repeating: " ", count: 4) }
        .joined(separator: "\r\n")
}

/// The draft's characters before the first `word`, as Left counts them.
private func index(of word: String, in draft: String) throws -> Int {
    draft.distance(from: draft.startIndex, to: try #require(draft.range(of: word)).lowerBound)
}

@Test func tapsCountCharactersOnOneRow() throws {
    let draft = "fix the typo here"
    let box = try #require(TerminalInputBox(try render(claude(["\u{276F}\u{A0}" + draft]))))
    #expect(box.top == 2 && box.bottom == 4)
    #expect(box.text == draft)
    // Columns are cells from the row's start; the draft begins after "❯ " at column 2.
    #expect(box.cursor(row: 3, column: 2.2)! == (0, draft.count))
    #expect(box.cursor(row: 3, column: 2.0 + 8.4)! == (8, draft.count))
    #expect(box.cursor(row: 3, column: 2.0 + 8.6)! == (9, draft.count))
    #expect(box.cursor(row: 3, column: 0)! == (0, draft.count))
    #expect(box.cursor(row: 3, column: 50)! == (draft.count, draft.count))
    #expect(box.cursor(row: 2, column: 5) == nil)
    #expect(box.cursor(row: 4, column: 5) == nil)
    #expect(box.cursor(row: 0, column: 3) == nil)
}

@Test func aRowTheMacWrappedCountsItsSpace() throws {
    let draft = "would it be possible to select where the cursor is to modify earlier parts"
    let rows = ["\u{276F}\u{A0}would it be possible to select where the", "  cursor is to modify earlier parts"]
    let box = try #require(TerminalInputBox(try render(claude(rows))))
    #expect(box.text == draft)
    let count = draft.count
    #expect(box.cursor(row: 4, column: 2)! == (try index(of: "cursor", in: draft), count))
    #expect(box.cursor(row: 4, column: 0)! == (try index(of: "cursor", in: draft), count))
    // Past the end of the first row: before the space Claude Code wrapped at.
    #expect(box.cursor(row: 3, column: 55)! == (try index(of: " cursor", in: draft), count))
    #expect(box.cursor(row: 4, column: 60)! == (count, count))
}

@Test func rowsThePhoneWrapsKeepTheirOrder() throws {
    let draft = "would it be possible to select where the cursor is to modify earlier oarts of a prompt"
    let frame = try render(claude(["\u{276F}\u{A0}would it be possible to select where the", "  cursor is to modify earlier oarts of a prompt"]), wrapColumns: 24)
    #expect(!frame.wrapContinuations.isEmpty)
    let box = try #require(TerminalInputBox(frame))
    #expect(box.text == draft)
    let typo = try index(of: "oarts", in: draft)
    let rows = (box.top + 1..<box.bottom).compactMap { row in
        (0..<24).lazy.compactMap { box.cursor(row: row, column: Double($0) + 0.2)?.index }.first { $0 == typo }.map { _ in row }
    }
    let row = try #require(rows.first)
    let column = try #require((0..<24).first { box.cursor(row: row, column: Double($0) + 0.2)?.index == typo })
    #expect(box.cursor(row: row, column: Double(column) + 0.8)! == (typo + 1, draft.count))
    // The last phone row of the first Mac row ends before the space the Mac wrapped at.
    let firstMacRowEnd = try index(of: " cursor", in: draft)
    #expect((box.top + 1..<box.bottom).contains { box.cursor(row: $0, column: 30)?.index == firstMacRowEnd })
    #expect(box.cursor(row: box.bottom - 1, column: 30)! == (draft.count, draft.count))
}

@Test func aWideCharacterIsOneCharacterAcrossTwoCells() throws {
    let box = try #require(TerminalInputBox(try render(claude(["\u{276F}\u{A0}\u{4E2D}\u{6587} ok"]))))
    #expect(box.cursor(row: 3, column: 2.9)! == (0, 5))
    #expect(box.cursor(row: 3, column: 3.1)! == (1, 5))
    #expect(box.cursor(row: 3, column: 4.9)! == (1, 5))
    #expect(box.cursor(row: 3, column: 5.1)! == (2, 5))
}

@Test func placeholdersHintsAndCommandsAreNotTapped() throws {
    let placeholder = try #require(TerminalInputBox(try render(claude(["\u{276F}\u{A0}\u{1B}[2mTry \"fix typecheck errors\"\u{1B}[0m"]))))
    #expect(placeholder.cursor(row: 3, column: 4) == nil)
    let command = try #require(TerminalInputBox(try render(claude(["\u{276F}\u{A0}/model"]))))
    #expect(command.cursor(row: 3, column: 4) == nil)
    let empty = try #require(TerminalInputBox(try render(claude(["\u{276F}\u{A0}"]))))
    #expect(empty.cursor(row: 3, column: 4) == nil)
}

@Test func noBoxWithoutBothRulesOrWithTheKeysBelowIt() throws {
    #expect(TerminalInputBox(try render("\u{276F}\u{A0}hello\r\n" + rule)) == nil)
    #expect(TerminalInputBox(try render(claude(["  \u{276F} 1. Yes"]))) == nil)
    #expect(TerminalInputBox(try render(claude(["\u{276F}\u{A0}hello"], below: ["\u{276F} agent row"]))) == nil)
    // The last box counts: a rule pair higher up in the transcript does not.
    let twice = claude(["\u{276F}\u{A0}old"]) + "\r\n" + claude(["\u{276F}\u{A0}new"])
    #expect(TerminalInputBox(try render(twice))?.text == "new")
}

@Test func theOffsetShowsTheBoxAboveTheKeyboardWithinTheContent() {
    // Rows end at 400, the box at 300; 500 points tall with 200 covered at the bottom.
    let bottom = 400.0 + 200 - 500
    #expect(TerminalInputBox.offset(showing: 300, insetBottom: 200, height: 500, minimum: -50, maximum: bottom) == 0)
    #expect(TerminalInputBox.offset(showing: 400, insetBottom: 200, height: 500, minimum: -50, maximum: bottom) == bottom)
    #expect(TerminalInputBox.offset(showing: 450, insetBottom: 200, height: 500, minimum: -50, maximum: bottom) == bottom)
    #expect(TerminalInputBox.offset(showing: 100, insetBottom: 200, height: 500, minimum: -50, maximum: bottom) == -50)
}
