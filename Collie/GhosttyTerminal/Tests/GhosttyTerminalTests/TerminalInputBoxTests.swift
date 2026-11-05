import Foundation
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
    let box = try #require(TerminalInputBox(try render(claude(["\u{276F}\u{A0}" + draft])), agent: "claude"))
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
    let box = try #require(TerminalInputBox(try render(claude(rows)), agent: "claude"))
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
    let box = try #require(TerminalInputBox(frame, agent: "claude"))
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
    let box = try #require(TerminalInputBox(try render(claude(["\u{276F}\u{A0}\u{4E2D}\u{6587} ok"])), agent: "claude"))
    #expect(box.cursor(row: 3, column: 2.9)! == (0, 5))
    #expect(box.cursor(row: 3, column: 3.1)! == (1, 5))
    #expect(box.cursor(row: 3, column: 4.9)! == (1, 5))
    #expect(box.cursor(row: 3, column: 5.1)! == (2, 5))
}

@Test func placeholdersHintsAndCommandsAreNotTapped() throws {
    let placeholder = try #require(TerminalInputBox(try render(claude(["\u{276F}\u{A0}\u{1B}[2mTry \"fix typecheck errors\"\u{1B}[0m"])), agent: "claude"))
    #expect(placeholder.cursor(row: 3, column: 4) == nil)
    let command = try #require(TerminalInputBox(try render(claude(["\u{276F}\u{A0}/model"])), agent: "claude"))
    #expect(command.cursor(row: 3, column: 4) == nil)
    let empty = try #require(TerminalInputBox(try render(claude(["\u{276F}\u{A0}"])), agent: "claude"))
    #expect(empty.cursor(row: 3, column: 4) == nil)
}

@Test func noBoxWithoutBothRulesOrWithTheKeysBelowIt() throws {
    #expect(TerminalInputBox(try render("\u{276F}\u{A0}hello\r\n" + rule), agent: "claude") == nil)
    #expect(TerminalInputBox(try render(claude(["  \u{276F} 1. Yes"])), agent: "claude") == nil)
    #expect(TerminalInputBox(try render(claude(["\u{276F}\u{A0}hello"], below: ["\u{276F} agent row"])), agent: "claude") == nil)
    // The last box counts: a rule pair higher up in the transcript does not.
    let twice = claude(["\u{276F}\u{A0}old"]) + "\r\n" + claude(["\u{276F}\u{A0}new"])
    #expect(TerminalInputBox(try render(twice), agent: "claude")?.text == "new")
}

@Test func theOffsetShowsTheBoxAboveTheKeyboardWithinTheContent() {
    // Rows end at 400, the box at 300; 500 points tall with 200 covered at the bottom.
    let bottom = 400.0 + 200 - 500
    #expect(TerminalInputBox.offset(showing: 300, insetBottom: 200, height: 500, minimum: -50, maximum: bottom) == 0)
    #expect(TerminalInputBox.offset(showing: 400, insetBottom: 200, height: 500, minimum: -50, maximum: bottom) == bottom)
    #expect(TerminalInputBox.offset(showing: 450, insetBottom: 200, height: 500, minimum: -50, maximum: bottom) == bottom)
    #expect(TerminalInputBox.offset(showing: 100, insetBottom: 200, height: 500, minimum: -50, maximum: bottom) == -50)
}

private func fixture(_ name: String) throws -> String {
    let url = URL(filePath: #filePath).deletingLastPathComponent()
        .appending(path: "../../../../crates/collied/tests/fixtures/\(name).ansi.txt")
    return try String(contentsOf: url, encoding: .utf8)
}

/// Whether some tap on the box's rows puts the cursor before the draft's `index`th character.
private func reaches(_ box: TerminalInputBox, _ index: Int, columns: Int = 220) -> Bool {
    (box.top + 1..<box.bottom).contains { row in (0..<columns).contains { box.cursor(row: row, column: Double($0) + 0.2)?.index == index } }
}

private let shade = "\u{1B}[48;2;34;34;34m"

/// Codex 0.166.0's screen around its input box, rows padded to the pane width.
private func codex(_ box: [String], above: String = " ") -> String {
    let pad = { (row: String) in "\u{1B}[0m" + shade + row + String(repeating: " ", count: 60) + "\u{1B}[0m" }
    let rows = ["\u{2022} Done.", above, pad(""), "\u{1B}[0m\u{1B}[1m" + shade + "\u{203A}" + pad(" " + box[0])]
    return (rows + box.dropFirst().map { pad("  " + $0) } + [pad(""), "  GPT-6.1-Sol high \u{B7} ~/src/demo"]).joined(separator: "\r\n")
}

/// Copilot CLI 1.0.105's screen around its input box.
private func copilot(_ box: [String], above: String = " ~/src/demo") -> String {
    let border = "\u{1B}[0m\u{1B}[38;2;164;117;249m"
    let top = border + "\u{257B}\u{1B}[0m\u{1B}[38;2;20;27;34m" + String(repeating: "\u{2584}", count: 59) + "\u{1B}[0m"
    let bottom = border + "\u{2579}\u{1B}[0m\u{1B}[38;2;20;27;34m" + String(repeating: "\u{2580}", count: 59) + "\u{1B}[0m"
    let rows = box.map { border + "\u{2503}\u{1B}[0m\u{1B}[48;2;20;27;34m " + $0 + String(repeating: " ", count: 20) + "\u{1B}[0m" }
    return (["\u{25CF} Done.", above, top] + rows + [bottom, " Autopilot \u{B7} Assisted Permissions"]).joined(separator: "\r\n")
}

@Test func codexsBoxCountsItsRowsAsLeftCrossesThem() throws {
    let draft = "Explain what calc.py does in one sentence"
    let one = try #require(TerminalInputBox(try render(try fixture("codex-0.160.1/draft")), agent: "codex"))
    #expect((one.top, one.bottom) == (55, 57))
    #expect(one.text == draft)
    // Codex's text begins after "› " at column 2.
    #expect(one.cursor(row: 56, column: 2.2)! == (0, draft.count))
    #expect(one.cursor(row: 56, column: 2.0 + 13.4)! == (13, draft.count))
    #expect(one.cursor(row: 56, column: 100)! == (draft.count, draft.count))
    #expect(one.cursor(row: 55, column: 5) == nil)
    let lines = try #require(TerminalInputBox(try render(try fixture("codex-0.160.1/draft-multiline")), agent: "codex"))
    let joined = draft + " and mention the return type."
    #expect(lines.text == joined)
    #expect(lines.cursor(row: 56, column: 2)! == (try index(of: "and", in: joined), joined.count))
    // A blank line between two lines is a newline of its own.
    let blank = try #require(TerminalInputBox(try render(try fixture("codex-0.160.1/draft-blank-line")), agent: "codex"))
    #expect(blank.cursor(row: 54, column: 2)! == ("first line of a draft".count + 2, blank.text.count))
    let working = try #require(TerminalInputBox(try render(try fixture("codex-0.160.1/working-draft")), agent: "codex"))
    #expect(working.text == "then list the files")
    let wrapped = try #require(TerminalInputBox(try render(try fixture("codex-0.160.1/draft-multiline"), wrapColumns: 24), agent: "codex"))
    #expect(wrapped.text == joined)
    for word in ["calc.py", "sentence", "and", "return", "type."] {
        #expect(reaches(wrapped, try index(of: word, in: joined), columns: 24), "\(word)")
    }
    #expect(reaches(wrapped, joined.count, columns: 24))
}

@Test func copilotsBoxCountsItsRowsAsLeftCrossesThem() throws {
    let one = try #require(TerminalInputBox(try render(try fixture("copilot-1.0.93-1/draft")), agent: "copilot"))
    #expect((one.top, one.bottom) == (56, 58))
    #expect(one.text == "Explain what calc.py does in one sentence")
    // Copilot CLI's text begins after "┃ " at column 2.
    #expect(one.cursor(row: 57, column: 2.2)! == (0, one.text.count))
    #expect(one.cursor(row: 57, column: 2.0 + 13.4)! == (13, one.text.count))
    let draft = "This is a long draft that I am typing on purpose so that it wraps over more than one row of the input box, "
        + "which lets the crop and the wrap tests see how Copilot lays out a draft taller than one row in a wide pane. "
        + "It keeps going for a while longer."
    let long = try #require(TerminalInputBox(try render(try fixture("copilot-1.0.93-1/draft-long")), agent: "copilot"))
    #expect(long.text == draft)
    #expect(long.cursor(row: 57, column: 2)! == (try index(of: "pane.", in: draft), draft.count))
    // Past the end of the first row: before the space Copilot CLI wrapped at.
    #expect(long.cursor(row: 56, column: 219)! == (try index(of: " pane.", in: draft), draft.count))
    let wrapped = try #require(TerminalInputBox(try render(try fixture("copilot-1.0.93-1/draft-long"), wrapColumns: 30), agent: "copilot"))
    #expect(wrapped.text == draft)
    for word in ["typing", "input", "wrap", "wide", "pane.", "longer."] {
        #expect(reaches(wrapped, try index(of: word, in: draft), columns: 30), "\(word)")
    }
    #expect(reaches(wrapped, draft.count, columns: 30))
    let empty = try #require(TerminalInputBox(try render(try fixture("copilot-1.0.93-1/idle")), agent: "copilot"))
    #expect(empty.cursor(row: 57, column: 2) == nil)
}

@Test func eachAgentFindsOnlyItsOwnBox() throws {
    let screens = ["claude": claude(["\u{276F}\u{A0}hello"]), "codex": codex(["hello"]), "copilot": copilot(["hello"])]
    for (agent, screen) in screens {
        for other in screens.keys {
            #expect((TerminalInputBox(try render(screen), agent: other)?.text == "hello") == (agent == other), "\(agent) \(other)")
        }
    }
    #expect(TerminalInputBox(try render(try fixture("codex-0.160.1/approval")), agent: "codex") == nil)
    #expect(TerminalInputBox(try render(codex(["hello"])), agent: "terminal") == nil)
}

@Test func codexsMenusPlaceholderScrollArrowsAndPastesAreNotTapped() throws {
    #expect(TerminalInputBox(try render(codex(["see @Coll"])), agent: "codex") != nil)
    #expect(TerminalInputBox(try render(codex(["see @Coll"], above: "  enter/tab insert \u{B7} esc close \u{B7} \u{2190}/\u{2192} filter")), agent: "codex") == nil)
    let popup = "\u{1B}[1m\u{1B}[48;2;99;168;248m\u{203A} /model  choose what model to use\u{1B}[0m"
    #expect(TerminalInputBox(try render(codex(["/mo"], above: popup)), agent: "codex") == nil)
    let placeholder = try #require(TerminalInputBox(try render(try fixture("codex-0.160.1/idle")), agent: "codex"))
    #expect(placeholder.cursor(row: 56, column: 4) == nil)
    let scrolled = try #require(TerminalInputBox(try render(codex(["word word" + String(repeating: " ", count: 20) + "\u{1B}[2m" + shade + "\u{2191}", "word"])), agent: "codex"))
    #expect(scrolled.cursor(row: 4, column: 3) == nil)
    let paste = try #require(TerminalInputBox(try render(codex(["\u{1B}[38;5;6m" + shade + "[Pasted Content 1507 chars]"])), agent: "codex"))
    #expect(paste.cursor(row: 3, column: 4) == nil)
    let plain = try #require(TerminalInputBox(try render(codex(["fix it"])), agent: "codex"))
    #expect(plain.cursor(row: 3, column: 4)! == (2, 6))
}

@Test func copilotsMenusAndSuggestionsAreNotTapped() throws {
    #expect(TerminalInputBox(try render(copilot(["see @Coll"])), agent: "copilot") != nil)
    #expect(TerminalInputBox(try render(copilot(["see @Coll"], above: "\u{2503}   @Collie/")), agent: "copilot") == nil)
    let grey = "\u{1B}[38;2;145;152;161m"
    let command = try #require(TerminalInputBox(try render(copilot(["/mo" + grey + "del "])), agent: "copilot"))
    #expect(command.cursor(row: 3, column: 3) == nil)
    let suggested = try #require(TerminalInputBox(try render(copilot(["fix " + grey + "the tests"])), agent: "copilot"))
    #expect(suggested.cursor(row: 3, column: 3) == nil)
    let plain = try #require(TerminalInputBox(try render(copilot(["fix it"])), agent: "copilot"))
    #expect(plain.cursor(row: 3, column: 4)! == (2, 6))
}
