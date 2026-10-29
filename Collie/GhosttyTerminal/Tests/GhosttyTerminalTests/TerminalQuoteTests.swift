#if canImport(UIKit)
import Testing
import UIKit

@testable import GhosttyTerminal

@MainActor
@Test func quoteHandsTheSelectedTextToItsCallback() {
    let view = GhosttyTerminalUIView(fontSize: 13)
    view.show(ansiSnapshot: "$ git status\r\nclean")
    var quoted: [String] = []
    view.onQuote = { quoted.append($0) }
    view.selection = TerminalSelection(TerminalCell(row: 0, column: 2), TerminalCell(row: 1, column: 4))
    view.quoteButton.sendActions(for: .primaryActionTriggered)
    #expect(quoted == ["git status\nclean"])
    #expect(view.selection == nil)
}

@MainActor
@Test func openHandsTheSelectedPathToItsCallback() throws {
    let view = GhosttyTerminalUIView(fontSize: 13)
    view.show(ansiSnapshot: "see https://a.dev or src/main.rs now")
    var opened: [String] = []
    let at = { (from: Int, to: Int) in TerminalSelection(TerminalCell(row: 0, column: from), TerminalCell(row: 0, column: to)) }
    view.selection = at(21, 28)
    view.showCopy(at: .zero)
    #expect(view.openButton.isHidden)
    var asked = 0
    view.filePath = { text in
        asked += 1
        return text.hasSuffix(".rs") ? text : nil
    }
    view.onOpenFile = { opened.append($0) }
    view.selection = at(21, 28)
    view.showCopy(at: .zero)
    #expect(!view.openButton.isHidden)
    view.openButton.sendActions(for: .primaryActionTriggered)
    #expect(opened == ["src/main.rs"])
    #expect(view.selection == nil)
    let before = asked
    view.selection = at(4, 28)
    view.showCopy(at: .zero)
    #expect(!view.openButton.isHidden)
    #expect(asked == before, "the link wins over the path")
    view.selection = nil

    let path = "/Users/me/git/chewtoy/Sources/App/Feature/Settings/Main.swift"
    let wrapped = GhosttyTerminalUIView(fontSize: 13)
    wrapped.frame = CGRect(x: 0, y: 0, width: 160, height: 800)
    wrapped.wraps = true
    wrapped.show(ansiSnapshot: "see \(path) ok")
    wrapped.layoutIfNeeded()
    let frame = try #require(wrapped.frameData)
    #expect(!frame.wrapContinuations.isEmpty)
    wrapped.filePath = { $0 }
    wrapped.onOpenFile = { opened.append($0) }
    let first = try #require(frame.runs.first { $0.text.contains("/Users") })
    let column = first.startColumn + first.text.distance(from: first.text.startIndex, to: try #require(first.text.range(of: "/Users")).lowerBound)
    wrapped.selection = TerminalSelection.word(at: TerminalCell(row: first.row, column: column), in: frame)
    #expect(wrapped.selection?.text(in: frame) != path)
    wrapped.showCopy(at: .zero)
    wrapped.openButton.sendActions(for: .primaryActionTriggered)
    #expect(opened.last == path)
}
#endif
