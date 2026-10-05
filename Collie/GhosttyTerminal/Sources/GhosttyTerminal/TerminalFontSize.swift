import CoreGraphics

enum TerminalFontSize {
    static let range: ClosedRange<CGFloat> = 7...24

    /// Whole points, so a pinch re-renders only when the size visibly changes.
    static func pinched(_ size: CGFloat, scale: CGFloat) -> CGFloat {
        min(max((size * scale).rounded(), range.lowerBound), range.upperBound)
    }
}
