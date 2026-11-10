import Foundation
import Testing

@testable import GhosttyTerminal

private func render(_ snapshot: String, wrapColumns: Int? = nil) throws -> TerminalFrame {
    let screen = try #require(TerminalScreen(background: TerminalRGB(0, 0, 0), foreground: TerminalRGB(255, 255, 255)))
    return screen.render(ansiSnapshot: snapshot, wrapColumns: wrapColumns)
}

private func inputBox(_ snapshot: String, agent: String, wrapColumns: Int? = nil) throws -> TerminalInputBox? {
    TerminalInputBox(try render(snapshot, wrapColumns: wrapColumns), agent: agent)
}

/// A tap's walk as `[lines, right, down, across]`, in a box the Mac draws as `mac` shows it.
private func keys(_ box: TerminalInputBox, _ row: Int, _ column: Double, mac: TerminalInputBox? = nil) -> [Int]? {
    box.walk(row: row, column: column, mac: mac ?? box).map { [$0.lines, $0.right, $0.down, $0.across] }
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
    let box = try #require(try inputBox(claude(["\u{276F}\u{A0}" + draft]), agent: "claude"))
    #expect(box.top == 2 && box.bottom == 4)
    #expect(box.text == draft)
    // Columns are cells from the row's start; the draft begins after "❯ " at column 2.
    #expect(keys(box, 3, 2.2) == [1, 0, 0, 0])
    #expect(keys(box, 3, 2.0 + 8.4) == [1, 0, 0, 8])
    #expect(keys(box, 3, 2.0 + 8.6) == [1, 0, 0, 9])
    #expect(keys(box, 3, 0) == [1, 0, 0, 0])
    #expect(keys(box, 3, 50) == [1, 0, 0, draft.count])
    #expect(keys(box, 2, 5) == nil)
    #expect(keys(box, 4, 5) == nil)
    #expect(keys(box, 0, 3) == nil)
}

@Test func aTapWalksDownToItsRowThenAcrossIt() throws {
    let draft = "would it be possible to select where the cursor is to modify earlier parts"
    let rows = ["\u{276F}\u{A0}would it be possible to select where the", "  cursor is to modify earlier parts"]
    let box = try #require(try inputBox(claude(rows), agent: "claude"))
    #expect(box.text == draft)
    // Past the end of the first row: before the space Claude Code wrapped at.
    #expect(keys(box, 3, 55) == [2, 0, 0, try index(of: " cursor", in: draft)])
    // Spaces typed at the end of the first row would not show; Down steps over them.
    #expect(keys(box, 4, 2) == [2, 0, 1, 0])
    #expect(keys(box, 4, 2.0 + 7.4) == [2, 0, 1, 7])
    #expect(keys(box, 4, 60) == [2, 0, 1, "cursor is to modify earlier parts".count])
}

@Test func rowsTheShownSnapshotJoinsWalkTheMacsRows() throws {
    let first = "would it be possible to select where the"
    let second = "cursor is to modify earlier parts"
    let mac = try #require(try inputBox(claude(["\u{276F}\u{A0}" + first, "  " + second, "  ok"]), agent: "claude"))
    let shown = try #require(try inputBox(claude(["\u{276F}\u{A0}" + first + " " + second, "  ok"]), agent: "claude"))
    // The joined row wraps where the Mac did not; Down counts the Mac's rows, and so does the walk to the start, since
    // collied also joins a typed newline right where the next word would not have fit.
    #expect(keys(shown, 3, 2.0 + 40.4, mac: mac) == [3, 0, 0, 40])
    #expect(keys(shown, 3, 2.0 + 41.4, mac: mac) == [3, 0, 1, 0])
    #expect(keys(shown, 3, 2.0 + 48.4, mac: mac) == [3, 0, 1, 7])
    #expect(keys(shown, 4, 2.0 + 1.4, mac: mac) == [3, 0, 2, 1])
    // A word split across rows joins without a space.
    let split = try #require(try inputBox(claude(["\u{276F}\u{A0}see " + String(repeating: "a", count: 52), "  aaaa ok"]), agent: "claude"))
    let joined = try #require(try inputBox(claude(["\u{276F}\u{A0}see " + String(repeating: "a", count: 56) + " ok"]), agent: "claude"))
    #expect(keys(joined, 3, 2.0 + 57.4, mac: split) == [2, 0, 1, 1])
    // Without the Mac's rows, or rows that do not match, the tap places nothing.
    #expect(keys(shown, 3, 2.0 + 48.4, mac: split) == nil)
    #expect(shown.walk(row: 3, column: 2.4, mac: nil) == nil)
}

@Test func rowsThePhoneWrapsKeepTheirOrder() throws {
    let draft = "would it be possible to selcet where the cursor is to modify earlier parts of a prompt"
    let screen = claude(["\u{276F}\u{A0}would it be possible to selcet where the", "  cursor is to modify earlier parts of a prompt"])
    #expect(!(try render(screen, wrapColumns: 24)).wrapContinuations.isEmpty)
    let box = try #require(try inputBox(screen, agent: "claude", wrapColumns: 24))
    #expect(box.text == draft)
    let typo = try index(of: "selcet", in: draft)
    let rows = (box.top + 1..<box.bottom).compactMap { row in
        (0..<24).lazy.compactMap { keys(box, row, Double($0) + 0.2)?[3] }.first { $0 == typo }.map { _ in row }
    }
    let row = try #require(rows.first)
    let column = try #require((0..<24).first { keys(box, row, Double($0) + 0.2)?[3] == typo })
    #expect(keys(box, row, Double(column) + 0.8) == [2, 0, 0, typo + 1])
    // The last phone row of the first Mac row ends before the space the Mac wrapped at.
    let firstMacRowEnd = try index(of: " cursor", in: draft)
    #expect((box.top + 1..<box.bottom).contains { keys(box, $0, 30) == [2, 0, 0, firstMacRowEnd] })
    #expect(keys(box, box.bottom - 1, 30) == [2, 0, 1, "cursor is to modify earlier parts of a prompt".count])
}

@Test func claudesSuggestionsAboveTheBoxKeepDownForThemselves() throws {
    let screen = (["\u{23FA} Done.", "    + Collie/", "    + Mac/CollieBar/", rule, "\u{276F}\u{A0}see @Coll and more", "  ok", rule])
        .joined(separator: "\r\n")
    let box = try #require(try inputBox(screen, agent: "claude"))
    #expect(keys(box, 4, 2.2) == [2, 0, 0, 0])
    #expect(keys(box, 5, 2.2) == nil)
    // A notice drawn at the right is no menu.
    let notice = (["\u{23FA} Done.", String(repeating: " ", count: 30) + "\u{25CF} high", rule, "\u{276F}\u{A0}see", "  ok", rule])
        .joined(separator: "\r\n")
    #expect(keys(try #require(try inputBox(notice, agent: "claude")), 4, 2.2) == [2, 0, 1, 0])
}

@Test func placeholdersHintsAndCommandsAreNotTapped() throws {
    let placeholder = try #require(try inputBox(claude(["\u{276F}\u{A0}\u{1B}[2mTry \"fix typecheck errors\"\u{1B}[0m"]), agent: "claude"))
    #expect(keys(placeholder, 3, 4) == nil)
    let command = try #require(try inputBox(claude(["\u{276F}\u{A0}/model"]), agent: "claude"))
    #expect(keys(command, 3, 4) == nil)
    let empty = try #require(try inputBox(claude(["\u{276F}\u{A0}"]), agent: "claude"))
    #expect(keys(empty, 3, 4) == nil)
}

@Test func noBoxWithoutBothRulesOrWithTheKeysBelowIt() throws {
    #expect(try inputBox("\u{276F}\u{A0}hello\r\n" + rule, agent: "claude") == nil)
    #expect(try inputBox(claude(["  \u{276F} 1. Yes"]), agent: "claude") == nil)
    #expect(try inputBox(claude(["\u{276F}\u{A0}hello"], below: ["\u{276F} agent row"]), agent: "claude") == nil)
    // The last box counts: a rule pair higher up in the transcript does not.
    let twice = claude(["\u{276F}\u{A0}old"]) + "\r\n" + claude(["\u{276F}\u{A0}new"])
    #expect(try inputBox(twice, agent: "claude")?.text == "new")
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

/// Whether some tap on the box's rows walks to just before `word` on the first of the Mac's
/// `rows` holding it. A Codex walk down steps right first, which its rows' first letter takes.
private func reaches(_ box: TerminalInputBox, _ word: String, on rows: [String], columns: Int = 220) throws -> Bool {
    let down = try #require(rows.firstIndex { $0.contains(word) })
    let across = try index(of: word, in: rows[down])
    return (box.top + 1..<box.bottom).contains { row in
        (0..<columns).contains { column in
            box.walk(row: row, column: Double(column) + 0.2, mac: box).map { ($0.down, $0.right + $0.across) } ?? (-1, 0) == (down, across)
        }
    }
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

@Test func codexsBoxWalksDownItsRows() throws {
    let draft = "Explain what calc.py does in one sentence"
    let one = try #require(try inputBox(try fixture("codex-0.160.1/draft"), agent: "codex"))
    #expect((one.top, one.bottom) == (55, 57))
    #expect(one.text == draft)
    // Codex's text begins after "› " at column 2.
    #expect(keys(one, 56, 2.2) == [1, 0, 0, 0])
    #expect(keys(one, 56, 2.0 + 13.4) == [1, 0, 0, 13])
    #expect(keys(one, 56, 100) == [1, 0, 0, draft.count])
    #expect(keys(one, 55, 5) == nil)
    let lines = try #require(try inputBox(try fixture("codex-0.160.1/draft-multiline"), agent: "codex"))
    let joined = draft + " and mention the return type."
    #expect(lines.text == joined)
    #expect(keys(lines, 55, 100) == [2, 0, 0, draft.count])
    // Spaces typed before the newline would not show; Down steps over them, after a Right.
    #expect(keys(lines, 56, 2) == [2, 1, 1, -1])
    #expect(keys(lines, 56, 2.0 + 4.4) == [2, 1, 1, 3])
    let blank = try #require(try inputBox(try fixture("codex-0.160.1/draft-blank-line"), agent: "codex"))
    #expect(keys(blank, 54, 2) == [3, 1, 2, -1])
    // A row that shows nothing may hold spaces Down would land after.
    #expect(keys(blank, 53, 2) == nil)
    let working = try #require(try inputBox(try fixture("codex-0.160.1/working-draft"), agent: "codex"))
    #expect(working.text == "then list the files")
    let wrapped = try #require(try inputBox(try fixture("codex-0.160.1/draft-multiline"), agent: "codex", wrapColumns: 24))
    #expect(wrapped.text == joined)
    for word in ["calc.py", "sentence", "return"] {
        #expect(try reaches(wrapped, word, on: [draft, "and mention the return type."], columns: 24), "\(word)")
    }
}

@Test func copilotsBoxWalksDownItsRows() throws {
    let one = try #require(try inputBox(try fixture("copilot-1.0.93-1/draft"), agent: "copilot"))
    #expect((one.top, one.bottom) == (56, 58))
    #expect(one.text == "Explain what calc.py does in one sentence")
    // Copilot CLI's text begins after "┃ " at column 2.
    #expect(keys(one, 57, 2.2) == [1, 0, 0, 0])
    #expect(keys(one, 57, 2.0 + 13.4) == [1, 0, 0, 13])
    let draft = "This is a long draft that I am typing on purpose so that it wraps over more than one row of the input box, "
        + "which lets the crop and the wrap tests see how Copilot lays out a draft taller than one row in a wide pane. "
        + "It keeps going for a while longer."
    let long = try #require(try inputBox(try fixture("copilot-1.0.93-1/draft-long"), agent: "copilot"))
    #expect(long.text == draft)
    // Past the end of the first row: before the space Copilot CLI wrapped at.
    #expect(keys(long, 56, 219) == [2, 0, 0, try index(of: " pane.", in: draft)])
    #expect(keys(long, 57, 2) == [2, 0, 1, 0])
    let wrapped = try #require(try inputBox(try fixture("copilot-1.0.93-1/draft-long"), agent: "copilot", wrapColumns: 30))
    #expect(wrapped.text == draft)
    let rows = [String(draft.prefix(try index(of: " pane.", in: draft))), "pane. It keeps going for a while longer."]
    for word in ["typing", "input", "wrap", "wide", "longer."] {
        #expect(try reaches(wrapped, word, on: rows, columns: 30), "\(word)")
    }
    let empty = try #require(try inputBox(try fixture("copilot-1.0.93-1/idle"), agent: "copilot"))
    #expect(keys(empty, 57, 2) == nil)
}

@Test func eachAgentFindsOnlyItsOwnBox() throws {
    let screens = ["claude": claude(["\u{276F}\u{A0}hello"]), "codex": codex(["hello"]), "copilot": copilot(["hello"])]
    for (agent, screen) in screens {
        for other in screens.keys {
            #expect((try inputBox(screen, agent: other)?.text == "hello") == (agent == other), "\(agent) \(other)")
        }
    }
    #expect(try inputBox(try fixture("codex-0.160.1/approval"), agent: "codex") == nil)
    #expect(try inputBox(codex(["hello"]), agent: "terminal") == nil)
}

@Test func codexsMenusPlaceholderScrollArrowsAndPastesAreNotTapped() throws {
    #expect(try inputBox(codex(["see @Coll"]), agent: "codex") != nil)
    #expect(try inputBox(codex(["see @Coll"], above: "  enter/tab insert \u{B7} esc close \u{B7} \u{2190}/\u{2192} filter"), agent: "codex") == nil)
    let popup = "\u{1B}[1m\u{1B}[48;2;99;168;248m\u{203A} /model  choose what model to use\u{1B}[0m"
    #expect(try inputBox(codex(["/mo"], above: popup), agent: "codex") == nil)
    let placeholder = try #require(try inputBox(try fixture("codex-0.160.1/idle"), agent: "codex"))
    #expect(keys(placeholder, 56, 4) == nil)
    let scrolled = try #require(try inputBox(codex(["word word" + String(repeating: " ", count: 20) + "\u{1B}[2m" + shade + "\u{2191}", "word"]), agent: "codex"))
    #expect(keys(scrolled, 4, 3) == nil)
    let paste = try #require(try inputBox(codex(["\u{1B}[38;5;6m" + shade + "[Pasted Content 1507 chars]"]), agent: "codex"))
    #expect(keys(paste, 3, 4) == nil)
    let plain = try #require(try inputBox(codex(["fix it"]), agent: "codex"))
    #expect(keys(plain, 3, 4) == [1, 0, 0, 2])
}

@Test func copilotsMenusAndSuggestionsAreNotTapped() throws {
    #expect(try inputBox(copilot(["see @Coll"]), agent: "copilot") != nil)
    #expect(try inputBox(copilot(["see @Coll"], above: "\u{2503}   @Collie/"), agent: "copilot") == nil)
    let grey = "\u{1B}[38;2;145;152;161m"
    let command = try #require(try inputBox(copilot(["/mo" + grey + "del "]), agent: "copilot"))
    #expect(keys(command, 3, 3) == nil)
    let suggested = try #require(try inputBox(copilot(["fix " + grey + "the tests"]), agent: "copilot"))
    #expect(keys(suggested, 3, 3) == nil)
    let plain = try #require(try inputBox(copilot(["fix it"]), agent: "copilot"))
    #expect(keys(plain, 3, 4) == [1, 0, 0, 2])
}

/// Each agent's screen with `rows` of text in its input box; the box wraps at 56 columns for
/// Claude Code (its 60-column rule less 4) and 57 for Codex and Copilot CLI (60 less 3).
private func screen(_ agent: String, _ rows: [String]) -> String {
    switch agent {
    case "claude": claude(["\u{276F}\u{A0}" + rows[0]] + rows.dropFirst().map { "  " + $0 })
    case "codex": codex(rows)
    default: copilot(rows)
    }
}

private let agents = ["claude", "codex", "copilot"]

/// The walk to `across` on Mac row `down` of a box of `lines` rows whose rows start with a
/// narrow character: Codex steps right before walking down.
private func walk(_ agent: String, lines: Int, down: Int, across: Int) -> [Int] {
    agent == "codex" && down > 0 ? [lines, 1, down, across - 1] : [lines, 0, down, across]
}

@Test(arguments: agents) func spacesTypedAtTheEndDoNotMoveTheTap(agent: String) throws {
    // The screen shows no spaces typed at the end; the walk reaches the start with Control-A.
    let box = try #require(try inputBox(screen(agent, ["hello   "]), agent: agent))
    #expect(keys(box, 3, 2.2) == [1, 0, 0, 0])
    #expect(keys(box, 3, 5.6) == [1, 0, 0, 4])
    #expect(keys(box, 3, 40) == [1, 0, 0, 5])
}

@Test(arguments: agents) func rowsAfterAFullRowWalkDown(agent: String) throws {
    let width = agent == "claude" ? 56 : 57
    let full = "see " + String(repeating: "a", count: width - 4)
    // The full row may end inside a word that continues on the next.
    let split = try #require(try inputBox(screen(agent, [full, String(repeating: "b", count: 10) + " ok"]), agent: agent))
    #expect(keys(split, 3, 2.2) == [2, 0, 0, 0])
    #expect(keys(split, 4, 2.2) == walk(agent, lines: 2, down: 1, across: 0))
    #expect(keys(split, 4, 40) == walk(agent, lines: 2, down: 1, across: 13))
    let short = try #require(try inputBox(screen(agent, ["see " + String(repeating: "a", count: width - 5), "ok go"]), agent: agent))
    #expect(keys(short, 4, 2.0 + 3.4) == walk(agent, lines: 2, down: 1, across: 3))
}

@Test(arguments: agents) func rowsAfterSpacesTheScreenDoesNotShowWalkDown(agent: String) throws {
    // `hello   ` and a newline: the screen shows `hello` alone, and Down steps over the spaces.
    let newline = try #require(try inputBox(screen(agent, ["hello   ", "world"]), agent: agent))
    #expect(keys(newline, 3, 2.2) == [2, 0, 0, 0])
    #expect(keys(newline, 3, 40) == [2, 0, 0, 5])
    #expect(keys(newline, 4, 2.2) == walk(agent, lines: 2, down: 1, across: 0))
    #expect(keys(newline, 4, 40) == walk(agent, lines: 2, down: 1, across: 5))
    let three = try #require(try inputBox(screen(agent, ["hello", "", "world"]), agent: agent))
    #expect(keys(three, 5, 2.0 + 2.4) == walk(agent, lines: 3, down: 2, across: 2))
}

@Test func copilotStartsTheRowAfterAFullOneWithItsSpace() throws {
    let full = "see " + String(repeating: "a", count: 53)
    let box = try #require(try inputBox(copilot([full, " ok go"]), agent: "copilot"))
    #expect(keys(box, 4, 3.2) == [2, 0, 1, 1])
    #expect(keys(box, 4, 2.2) == [2, 0, 1, 0])
}

@Test(arguments: agents) func wideCharactersCountOnce(agent: String) throws {
    let one = try #require(try inputBox(screen(agent, ["\u{4E2D}\u{6587} ok"]), agent: agent))
    #expect(keys(one, 3, 2.9) == [1, 0, 0, 0])
    #expect(keys(one, 3, 3.1) == [1, 0, 0, 1])
    #expect(keys(one, 3, 5.1) == [1, 0, 0, 2])
    let width = agent == "claude" ? 56 : 57
    let cjk = String(repeating: "\u{4E2D}", count: width / 2)
    let split = try #require(try inputBox(screen(agent, [cjk, "\u{6587}\u{5B57} ok"]), agent: agent))
    #expect(keys(split, 3, 3.1) == [2, 0, 0, 1])
    // Codex steps right over the first row's wide character, and Down keeps its two cells.
    #expect(keys(split, 4, 2.2) == (agent == "codex" ? [2, 1, 1, -1] : [2, 0, 1, 0]))
    #expect(keys(split, 4, 5.1) == (agent == "codex" ? [2, 1, 1, 1] : [2, 0, 1, 2]))
    let narrow = try #require(try inputBox(screen(agent, ["ok", "\u{6587}\u{5B57} ok"]), agent: agent))
    #expect(keys(narrow, 4, 3.1) == (agent == "codex" ? [2, 1, 1, 1] : [2, 0, 1, 1]))
}

@Test func codexsWalkDownNeedsARowAsWideAsItsFirstStep() throws {
    #expect(keys(try #require(try inputBox(codex(["", "ok"]), agent: "codex")), 4, 2.2) == nil)
    let wide = try #require(try inputBox(codex(["\u{4E2D} ok", "a"]), agent: "codex"))
    #expect(keys(wide, 4, 2.2) == nil)
}

@Test func copilotsBoxOfEightRowsOrMoreMayBeScrolled() throws {
    let seven = try #require(try inputBox(copilot(Array(repeating: "word", count: 7)), agent: "copilot"))
    #expect(keys(seven, 3, 4.2) == [7, 0, 0, 2])
    #expect(keys(seven, 9, 4.2) == [7, 0, 6, 2])
    let eight = try #require(try inputBox(copilot(Array(repeating: "word", count: 8)), agent: "copilot"))
    #expect(keys(eight, 3, 2.2) == nil)
}
