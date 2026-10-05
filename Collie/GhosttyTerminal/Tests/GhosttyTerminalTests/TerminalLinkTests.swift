import Testing

@testable import GhosttyTerminal

private func link(_ text: String, touching part: String) throws -> String? {
    let found = try #require(text.range(of: part))
    let scalars = text.unicodeScalars
    let lower = scalars.distance(from: scalars.startIndex, to: found.lowerBound)
    let upper = scalars.distance(from: scalars.startIndex, to: found.upperBound)
    return TerminalLink.url(in: Array(scalars), touching: lower..<upper)?.absoluteString
}

private func render(_ snapshot: String, wrapColumns: Int? = nil) throws -> TerminalFrame {
    let screen = try #require(TerminalScreen(background: TerminalRGB(0, 0, 0), foreground: TerminalRGB(255, 255, 255)))
    return screen.render(ansiSnapshot: snapshot, wrapColumns: wrapColumns)
}

private func selection(_ row: Int, _ column: Int, _ endRow: Int, _ endColumn: Int) -> TerminalSelection {
    TerminalSelection(TerminalCell(row: row, column: column), TerminalCell(row: endRow, column: endColumn))
}

@Test func linkAroundAPartialSelection() throws {
    let text = "see https://example.com/a/b?q=1&r=(2)#top for more"
    #expect(try link(text, touching: "ample") == "https://example.com/a/b?q=1&r=(2)#top")
    #expect(try link(text, touching: "https") == "https://example.com/a/b?q=1&r=(2)#top")
    #expect(try link(text, touching: "top for") == "https://example.com/a/b?q=1&r=(2)#top")
    #expect(try link(text, touching: "see") == nil)
    #expect(try link(text, touching: " for") == nil)
    #expect(try link("HTTP://Example.com/X", touching: "Example") == "HTTP://Example.com/X")
}

@Test func linkDropsTrailingPunctuation() throws {
    #expect(try link("open https://a.dev/x.", touching: "a.dev") == "https://a.dev/x")
    #expect(try link("https://a.dev/x, https://b.dev", touching: "a.dev") == "https://a.dev/x")
    #expect(try link("at https://a.dev/x: done!", touching: "a.dev") == "https://a.dev/x")
    #expect(try link("'https://a.dev/x'", touching: "a.dev") == "https://a.dev/x")
    #expect(try link("\"https://a.dev/x\"", touching: "a.dev") == "https://a.dev/x")
    #expect(try link("<https://a.dev/x>", touching: "a.dev") == "https://a.dev/x")
    #expect(try link("`https://a.dev/x`", touching: "a.dev") == "https://a.dev/x")
    #expect(try link("see https://localhost/x. Done", touching: ". D") == nil)
}

@Test func linkKeepsOnlyBalancedClosingBrackets() throws {
    #expect(try link("(see https://a.dev/x)", touching: "a.dev") == "https://a.dev/x")
    #expect(try link("[docs](https://a.dev/x).", touching: "a.dev") == "https://a.dev/x")
    #expect(try link("(https://en.wikipedia.org/wiki/Foo_(bar)).", touching: "wiki") == "https://en.wikipedia.org/wiki/Foo_(bar)")
    #expect(try link("https://a.dev/x[1]", touching: "a.dev") == "https://a.dev/x%5B1%5D")
    #expect(try link("[https://a.dev/x]", touching: "a.dev") == "https://a.dev/x")
    #expect(try link("https://a.dev/" + String(repeating: ")", count: 20000), touching: "a.dev") == "https://a.dev/")
}

@Test func linkIsOnlyHttpOrHttps() throws {
    #expect(try link("ftp://a.dev/x", touching: "a.dev") == nil)
    #expect(try link("file:///etc/hosts", touching: "etc") == nil)
    #expect(try link("javascript:alert(1)//https", touching: "alert") == nil)
    #expect(try link("mailto:me@a.dev", touching: "a.dev") == nil)
    #expect(try link("xhttps://a.dev", touching: "a.dev") == nil)
    #expect(try link("2https://a.dev", touching: "a.dev") == nil)
    #expect(try link("\u{53C2}\u{7167}https://a.dev/x", touching: "a.dev") == "https://a.dev/x")
    #expect(try link("https://github.com@evil.example/login", touching: "github") == nil)
    #expect(try link("https://user:token@a.dev/x", touching: "a.dev") == nil)
    #expect(try link("https://", touching: "https") == nil)
    #expect(try link("https://.", touching: "https") == nil)
    #expect(try link("https:/a.dev", touching: "a.dev") == nil)
    #expect(try link("https\u{FF1A}//a.dev", touching: "a.dev") == nil)
}

@Test func linkStopsAtSpacesAndControlCharacters() throws {
    #expect(try link("https://a.dev/x y", touching: "y") == nil)
    #expect(try link("https://a.dev/x y", touching: "a.dev") == "https://a.dev/x")
    #expect(try link("https://a.dev/x\u{7}y", touching: "y") == nil)
    #expect(try link("https://a.dev/x\u{7}y", touching: "a.dev") == "https://a.dev/x")
    #expect(try link("https://a.dev/x\ty", touching: "y") == nil)
    #expect(try link("https://a.dev/x\u{A0}y", touching: "y") == nil)
    #expect(try link("https://a.dev/caf\u{E9}", touching: "a.dev") == nil)
    #expect(try link("https://b\u{FC}cher.de/x", touching: "cher") == nil)
    #expect(try link("https://a.dev/pull/123\u{2026}", touching: "a.dev") == nil)
    #expect(try link("https://a.dev/x \u{53C2}\u{7167}", touching: "a.dev") == "https://a.dev/x")
    #expect(try link("https://a.dev/x\nhttps://b.dev", touching: "x\nhttps://b") == nil)
}

@Test func linkIsOneUrl() throws {
    #expect(try link("https://a.dev https://b.dev", touching: "dev https") == nil)
    #expect(try link("https://a.dev and https://a.dev", touching: "dev and https") == "https://a.dev")
}

@Test func linkAcrossSoftWrappedRows() throws {
    let frame = try render("see https://example.com/a/very/long/path ok\r\nnext", wrapColumns: 12)
    #expect(frame.wrapContinuations == [1, 2, 3])
    let url = "https://example.com/a/very/long/path"
    #expect(selection(2, 3, 2, 3).link(in: frame)?.absoluteString == url)
    #expect(selection(0, 4, 0, 5).link(in: frame)?.absoluteString == url)
    #expect(selection(3, 0, 3, 1).link(in: frame)?.absoluteString == url)
    #expect(selection(0, 0, 4, 3).link(in: frame)?.absoluteString == url)
    #expect(selection(0, 0, 0, 2).link(in: frame) == nil)
    #expect(selection(3, 5, 3, 6).link(in: frame) == nil)
    #expect(selection(4, 0, 4, 3).link(in: frame) == nil)
}

@Test func linkEndsAtAHardLineBreak() throws {
    let frame = try render("https://example.com/a\r\nb/c")
    #expect(selection(0, 3, 0, 3).link(in: frame)?.absoluteString == "https://example.com/a")
    #expect(selection(1, 0, 1, 0).link(in: frame) == nil)
    #expect(selection(0, 0, 9, 9).link(in: frame)?.absoluteString == "https://example.com/a")
}

@Test func linkSkipsHiddenText() throws {
    let frame = try render("go https://good.example\u{1B}[8m.evil.example/login\u{1B}[28m now")
    #expect(selection(0, 12, 0, 12).link(in: frame)?.absoluteString == "https://good.example")
    #expect(selection(0, 30, 0, 30).link(in: frame) == nil)
    #expect(selection(0, 10, 0, 10).link(in: try render("click \u{1B}[8mhttps://evil.example")) == nil)
    #expect(selection(0, 10, 0, 10).link(in: try render("click \u{1B}[38;2;0;0;0mhttps://evil.example")) == nil)
}

@Test func linkEndingOnAFullRowIsNotOfferedOnceWrapsAreUnknown() throws {
    let filler = Array(repeating: "x", count: 520).joined(separator: "\r\n")
    let frame = try render(filler + "\r\nsee https://example.com/a/very/long/path ok\r\nhttps://a.dev ok", wrapColumns: 24)
    #expect(frame.wrapsUnknown)
    #expect(frame.rows == 500)
    #expect(selection(497, 10, 497, 10).link(in: frame) == nil)
    #expect(selection(499, 3, 499, 3).link(in: frame)?.absoluteString == "https://a.dev")
}
