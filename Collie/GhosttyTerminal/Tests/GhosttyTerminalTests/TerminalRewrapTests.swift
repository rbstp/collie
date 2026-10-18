#if canImport(UIKit)
import Testing
import UIKit

@testable import GhosttyTerminal

@MainActor
private func terminal() -> GhosttyTerminalUIView {
    let view = GhosttyTerminalUIView(fontSize: 13)
    view.wraps = true
    view.frame = CGRect(x: 0, y: 0, width: 320, height: 300)
    view.layoutIfNeeded()
    // Long lines first, so their rows shrink on a wider view while the short ones below do not.
    view.show(ansiSnapshot: (0..<80).map { n in
        "line \(n): " + (n < 30 ? String(repeating: "wrapped words ", count: 12) : "short")
    }.joined(separator: "\r\n"))
    view.layoutIfNeeded()
    return view
}

@MainActor
private func bottom(_ view: GhosttyTerminalUIView) -> CGFloat {
    max(view.contentSize.height + view.adjustedContentInset.bottom - view.bounds.height, -view.adjustedContentInset.top)
}

@MainActor
private func topLine(_ view: GhosttyTerminalUIView) throws -> String {
    let m = CellMetrics(size: 13)
    let frame = try #require(view.frameData)
    let row = max(Int(((view.contentOffset.y + view.adjustedContentInset.top - m.inset) / m.height).rounded()), 0)
    #expect(!frame.wrapContinuations.contains(row))
    return frame.runs.filter { $0.row == row }.sorted { $0.startColumn < $1.startColumn }.map(\.text).joined()
}

@MainActor
private func widen(_ view: GhosttyTerminalUIView) throws {
    let columns = try #require(view.frameData).columns
    view.frame.size.width = 700
    view.layoutIfNeeded()
    #expect(try #require(view.frameData).columns > columns)
}

@MainActor
@Test func rewrapKeepsFollowingTheBottom() throws {
    let view = terminal()
    #expect(view.contentOffset.y == bottom(view))
    try widen(view)
    #expect(view.contentOffset.y == bottom(view))
}

@MainActor
@Test func rewrapKeepsTheLineAtTheTop() throws {
    let view = terminal()
    let frame = try #require(view.frameData)
    let row = try #require((0..<frame.rows).first { row in
        frame.runs.contains { $0.row == row && $0.startColumn == 0 && $0.text.hasPrefix("line 40:") }
    })
    let m = CellMetrics(size: 13)
    view.contentOffset.y = m.inset + CGFloat(row) * m.height - view.adjustedContentInset.top
    view.delegate?.scrollViewDidEndDecelerating?(view)
    try widen(view)
    #expect(try topLine(view).hasPrefix("line 40:"))
    #expect(view.contentOffset.y < bottom(view))
}

@MainActor
@Test func rewrapKeepsAScrollToTheTop() throws {
    let view = terminal()
    view.contentOffset.y = -view.adjustedContentInset.top
    view.delegate?.scrollViewDidScrollToTop?(view)
    try widen(view)
    #expect(view.contentOffset.y == -view.adjustedContentInset.top)
    #expect(try topLine(view).hasPrefix("line 0:"))
}

@MainActor
@Test func obscuredTopKeepsTheFirstRowReachable() throws {
    let view = terminal()
    view.obscuredTop = 100
    view.layoutIfNeeded()
    #expect(view.contentOffset.y == bottom(view))
    #expect(view.adjustedContentInset.top == 100)

    view.show(ansiSnapshot: "one\r\ntwo")
    view.layoutIfNeeded()
    #expect(view.contentOffset.y + view.bounds.height == view.contentSize.height)
}
#endif
