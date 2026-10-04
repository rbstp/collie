import SwiftUI

/// SF Symbols has no wrap-text glyph; this one is drawn in its style.
struct WrapLinesShape: Shape {
    func path(in r: CGRect) -> Path {
        let s = min(r.width, r.height)
        let o = CGPoint(x: r.midX - s / 2, y: r.midY - s / 2)
        func pt(_ x: CGFloat, _ y: CGFloat) -> CGPoint { CGPoint(x: o.x + x * s, y: o.y + y * s) }
        var p = Path()
        p.move(to: pt(0.10, 0.22))
        p.addLine(to: pt(0.90, 0.22))
        p.move(to: pt(0.10, 0.50))
        p.addLine(to: pt(0.74, 0.50))
        p.addArc(center: pt(0.74, 0.64), radius: 0.14 * s, startAngle: .degrees(-90), endAngle: .degrees(90), clockwise: false)
        p.addLine(to: pt(0.56, 0.78))
        p.move(to: pt(0.64, 0.70))
        p.addLine(to: pt(0.56, 0.78))
        p.addLine(to: pt(0.64, 0.86))
        p.move(to: pt(0.10, 0.78))
        p.addLine(to: pt(0.36, 0.78))
        return p
    }
}
