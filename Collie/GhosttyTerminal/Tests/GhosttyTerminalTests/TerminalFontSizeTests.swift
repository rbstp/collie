import Testing

@testable import GhosttyTerminal

@Test func pinchSnapsToWholePoints() {
    #expect(TerminalFontSize.pinched(11, scale: 1) == 11)
    #expect(TerminalFontSize.pinched(11, scale: 1.04) == 11)
    #expect(TerminalFontSize.pinched(11, scale: 1.05) == 12)
    #expect(TerminalFontSize.pinched(11, scale: 0.85) == 9)
}

@Test func pinchStaysWithinTheRange() {
    #expect(TerminalFontSize.pinched(11, scale: 0.1) == 7)
    #expect(TerminalFontSize.pinched(11, scale: 10) == 24)
    #expect(TerminalFontSize.pinched(24, scale: 1.2) == 24)
}
