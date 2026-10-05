import CoreGraphics

public enum TerminalSwipe: Sendable {
    case left
    case right

    static func ended(translation: CGPoint, velocity: CGPoint) -> TerminalSwipe? {
        let distance = abs(translation.x)
        guard distance > 2 * abs(translation.y), translation.x * velocity.x >= 0,
            distance >= 100 || (distance >= 40 && abs(velocity.x) >= 500)
        else { return nil }
        return translation.x < 0 ? .left : .right
    }
}
