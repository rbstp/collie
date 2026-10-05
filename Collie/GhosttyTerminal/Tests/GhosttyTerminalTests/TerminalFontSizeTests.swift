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

@Test func pinchKeepsThePointUnderTheFingers() {
    #expect(TerminalFontSize.pinchedOffset(2300, offset: 2000, from: 3000, to: 6000, viewport: 600, leading: 0, trailing: 0) == 4300)
    #expect(TerminalFontSize.pinchedOffset(2300, offset: 2000, from: 3000, to: 1500, viewport: 600, leading: 0, trailing: 0) == 850)
}

@Test func pinchKeepsTheViewportWithinTheContent() {
    #expect(TerminalFontSize.pinchedOffset(2590, offset: 2400, from: 3000, to: 1900, viewport: 600, leading: 0, trailing: 34) == 1334)
    #expect(TerminalFontSize.pinchedOffset(100, offset: 0, from: 3000, to: 1500, viewport: 600, leading: 20, trailing: 0) == -20)
    #expect(TerminalFontSize.pinchedOffset(100, offset: 0, from: 800, to: 400, viewport: 600, leading: 20, trailing: 0) == -20)
}
