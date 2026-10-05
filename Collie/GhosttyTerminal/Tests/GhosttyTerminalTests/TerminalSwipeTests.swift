import CoreGraphics
import Testing

@testable import GhosttyTerminal

@Test func aLongOrFastSidewaysPanIsASwipe() {
    #expect(TerminalSwipe.ended(translation: CGPoint(x: -120, y: 10), velocity: .zero) == .left)
    #expect(TerminalSwipe.ended(translation: CGPoint(x: 120, y: -10), velocity: .zero) == .right)
    #expect(TerminalSwipe.ended(translation: CGPoint(x: 50, y: 0), velocity: CGPoint(x: 800, y: 0)) == .right)
}

@Test func aShortSlowOrMostlyVerticalPanIsNot() {
    #expect(TerminalSwipe.ended(translation: CGPoint(x: 60, y: 0), velocity: CGPoint(x: 200, y: 0)) == nil)
    #expect(TerminalSwipe.ended(translation: CGPoint(x: 30, y: 0), velocity: CGPoint(x: 2000, y: 0)) == nil)
    #expect(TerminalSwipe.ended(translation: CGPoint(x: 150, y: 80), velocity: .zero) == nil)
    #expect(TerminalSwipe.ended(translation: CGPoint(x: 0, y: 300), velocity: CGPoint(x: 0, y: 1500)) == nil)
}

@Test func aSwipeFlungBackIsCancelled() {
    #expect(TerminalSwipe.ended(translation: CGPoint(x: 120, y: 0), velocity: CGPoint(x: -300, y: 0)) == nil)
}
