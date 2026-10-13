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
#endif
