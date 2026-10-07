import AppKit

enum MenuIcon {
    /// A template image, so the menu bar tints it: full alpha while collied runs, dim when
    /// off, and a dot at the top right while an approval is pending.
    static func image(running: Bool, pending: Bool) -> NSImage {
        let size = NSSize(width: 22, height: 18)
        let glyph = NSImage(named: "MenuIcon")
        let image = NSImage(size: size, flipped: false) { rect in
            glyph?.draw(
                in: NSRect(x: (rect.width - 18) / 2 - 1, y: 0, width: 18, height: 18), from: .zero,
                operation: .sourceOver, fraction: running ? 1 : 0.35)
            if pending {
                let dot = NSRect(x: rect.maxX - 7, y: rect.maxY - 7, width: 6, height: 6)
                NSGraphicsContext.current?.compositingOperation = .clear
                NSBezierPath(ovalIn: dot.insetBy(dx: -1.5, dy: -1.5)).fill()
                NSGraphicsContext.current?.compositingOperation = .sourceOver
                NSColor.black.setFill()
                NSBezierPath(ovalIn: dot).fill()
            }
            return true
        }
        image.isTemplate = true
        return image
    }

    static func label(_ state: DaemonState) -> String {
        switch state {
        case .off: "collie: off"
        case .outdated: "collie: running"
        case .running(let n) where n > 0: "collie: running, \(pendingText(n))"
        case .running: "collie: running"
        }
    }

    static func pendingText(_ n: Int) -> String {
        n == 1 ? "1 approval pending" : "\(n) approvals pending"
    }
}
