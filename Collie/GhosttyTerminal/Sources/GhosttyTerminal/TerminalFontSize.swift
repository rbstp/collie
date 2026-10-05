import CoreGraphics

enum TerminalFontSize {
    static let range: ClosedRange<CGFloat> = 7...24

    /// Whole points, so a pinch re-renders only when the size visibly changes.
    static func pinched(_ size: CGFloat, scale: CGFloat) -> CGFloat {
        min(max((size * scale).rounded(), range.lowerBound), range.upperBound)
    }

    /// Keeps the content under the pinch at the same place on screen, and the viewport within
    /// the content: a scroll view does not clamp its offset when the content shrinks.
    static func pinchedOffset(
        _ point: CGFloat, offset: CGFloat, from old: CGFloat, to new: CGFloat,
        viewport: CGFloat, leading: CGFloat, trailing: CGFloat
    ) -> CGFloat {
        let anchored = point / old * new - (point - offset)
        return min(max(anchored, -leading), max(new + trailing - viewport, -leading))
    }
}
