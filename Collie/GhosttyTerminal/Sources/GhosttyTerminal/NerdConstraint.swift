import CoreGraphics

/// Ghostty's glyph constraint (`Glyph.RenderOptions.Constraint` in `src/font/Glyph.zig` at the
/// commit `scripts/ghostty/build-xcframework.sh` pins), which sizes and aligns Nerd Font glyphs
/// to the cell grid. Rectangles are in points from the cell's bottom-left corner, y up.
struct NerdConstraint: Equatable {
    enum Size { case none, fit, cover, fitCover1, stretch }
    enum Align { case none, start, end, center, center1 }

    struct Metrics {
        var cellWidth: CGFloat
        var cellHeight: CGFloat
        var faceWidth: CGFloat
        var faceHeight: CGFloat
        var faceY: CGFloat
        var iconHeight: CGFloat
        var iconHeightSingle: CGFloat
    }

    var size = Size.none
    var iconHeight = false
    var maxConstraintWidth = 2
    var alignHorizontal = Align.none
    var alignVertical = Align.none
    var padTop: CGFloat = 0
    var padLeft: CGFloat = 0
    var padRight: CGFloat = 0
    var padBottom: CGFloat = 0
    var relativeWidth: CGFloat = 1
    var relativeHeight: CGFloat = 1
    var relativeX: CGFloat = 0
    var relativeY: CGFloat = 0
    var maxXYRatio: CGFloat?

    static func lookup(_ scalar: UInt32) -> NerdConstraint? {
        var low = 0
        var high = ranges.count / 3 - 1
        while low <= high {
            let mid = (low + high) / 2
            if scalar < ranges[mid * 3] {
                high = mid - 1
            } else if scalar > ranges[mid * 3 + 1] {
                low = mid + 1
            } else {
                return all[Int(ranges[mid * 3 + 2])]
            }
        }
        return nil
    }

    func constrain(_ glyph: CGRect, _ metrics: Metrics, constraintWidth: Int) -> CGRect {
        guard size != .none || alignHorizontal != .none || alignVertical != .none else { return glyph }
        guard size == .stretch else { return inner(glyph, metrics, constraintWidth) }
        var m = metrics
        m.faceWidth = m.cellWidth
        m.faceHeight = m.cellHeight
        m.faceY = 0
        var c = self
        c.padTop = max(0, padTop)
        c.padLeft = max(0, padLeft)
        c.padRight = max(0, padRight)
        c.padBottom = max(0, padBottom)
        return c.inner(glyph, m, constraintWidth)
    }

    private func inner(_ glyph: CGRect, _ m: Metrics, _ constraintWidth: Int) -> CGRect {
        let cells = size == .stretch && m.faceWidth > 0.9 * m.faceHeight ? 1 : min(maxConstraintWidth, constraintWidth)
        let groupWidth = glyph.width / relativeWidth
        let groupHeight = glyph.height / relativeHeight
        var group = CGRect(
            x: glyph.minX - groupWidth * relativeX, y: glyph.minY - groupHeight * relativeY,
            width: groupWidth, height: groupHeight
        )
        let (widthFactor, heightFactor) = scaleFactors(group, m, cells)
        let center = CGPoint(x: group.midX, y: group.midY)
        group.size = CGSize(width: group.width * widthFactor, height: group.height * heightFactor)
        group.origin = CGPoint(x: center.x - group.width / 2, y: center.y - group.height / 2)
        group.origin = CGPoint(x: alignedX(group, m, cells), y: alignedY(group, m))
        return CGRect(
            x: group.minX + group.width * relativeX, y: group.minY + group.height * relativeY,
            width: widthFactor * glyph.width, height: heightFactor * glyph.height
        )
    }

    private func scaleFactors(_ group: CGRect, _ m: Metrics, _ cells: Int) -> (CGFloat, CGFloat) {
        guard size != .none else { return (1, 1) }
        let multiCell = cells > 1
        let targetWidth = (CGFloat(cells) - (padLeft + padRight)) * m.faceWidth
        let targetHeight = (1 - (padBottom + padTop)) * (iconHeight ? (multiCell ? m.iconHeight : m.iconHeightSingle) : m.faceHeight)
        var widthFactor = targetWidth / group.width
        var heightFactor = targetHeight / group.height
        switch size {
        case .fit:
            heightFactor = min(1, widthFactor, heightFactor)
            widthFactor = heightFactor
        case .cover:
            heightFactor = min(widthFactor, heightFactor)
            widthFactor = heightFactor
        case .fitCover1:
            heightFactor = min(widthFactor, heightFactor)
            if multiCell && heightFactor > 1 {
                heightFactor = max(1, scaleFactors(group, m, 1).1)
            }
            widthFactor = heightFactor
        case .stretch, .none:
            break
        }
        if let maxXYRatio, group.width * widthFactor > group.height * heightFactor * maxXYRatio {
            widthFactor = group.height * heightFactor * maxXYRatio / group.width
        }
        return (widthFactor, heightFactor)
    }

    private func alignedY(_ group: CGRect, _ m: Metrics) -> CGFloat {
        if size == .none && alignVertical == .none { return group.minY }
        let start = m.faceY + padBottom * m.faceHeight
        let end = m.faceY + (m.faceHeight - group.height - padTop * m.faceHeight)
        let center = (start + end) / 2
        switch alignVertical {
        case .none: return end < start ? center : max(start, min(group.minY, end))
        case .start: return start
        case .end: return end
        case .center, .center1: return center
        }
    }

    private func alignedX(_ group: CGRect, _ m: Metrics, _ cells: Int) -> CGFloat {
        if size == .none && alignHorizontal == .none { return group.minX }
        let span = m.faceWidth + CGFloat(cells - 1) * m.cellWidth
        let start = padLeft * m.faceWidth
        let end = span - group.width - padRight * m.faceWidth
        switch alignHorizontal {
        case .none: return max(start, min(group.minX, end))
        case .start: return start
        case .end: return max(start, end)
        case .center: return max(start, (start + end) / 2)
        case .center1: return max(start, (start + m.faceWidth - group.width - padRight * m.faceWidth) / 2)
        }
    }
}

/// Ghostty's cell rules for constrained glyphs (`src/renderer/cell.zig` at the same commit).
enum GhosttyCell {
    static func isSymbol(_ cp: UInt32) -> Bool {
        switch cp {
        case 0xE000...0xF8FF, 0xF0000...0xFFFFD, 0x100000...0x10FFFD,  // Private Use
            0x2190...0x21FF,  // Arrows
            0x2460...0x24FF,  // Enclosed Alphanumerics
            0x2600...0x26FF,  // Miscellaneous Symbols
            0x2700...0x27BF,  // Dingbats
            0x1F100...0x1F1FF,  // Enclosed Alphanumeric Supplement
            0x1F300...0x1F5FF,  // Miscellaneous Symbols and Pictographs
            0x1F600...0x1F64F,  // Emoticons
            0x1F680...0x1F6FF:  // Transport and Map Symbols
            true
        default: false
        }
    }

    static func isGraphicsElement(_ cp: UInt32) -> Bool {
        switch cp {
        case 0x2500...0x259F, 0x1FB00...0x1FBFF, 0x1CC00...0x1CEBF, 0xE0B0...0xE0D7: true
        default: false
        }
    }

    /// How many cells the glyph in `row[x]` may cover, from the first code point of each cell in the
    /// row (0 for a cell never written). Ghostty also holds the screen's last column to one cell,
    /// which a snapshot cannot tell apart from the last written one.
    static func constraintWidth(_ row: [UInt32], _ x: Int) -> Int {
        guard row.indices.contains(x), isSymbol(row[x]) else { return 1 }
        if x > 0 && isSymbol(row[x - 1]) && !isGraphicsElement(row[x - 1]) { return 1 }
        let next = x + 1 < row.count ? row[x + 1] : 0
        return next == 0 || next == 0x20 || next == 0x2002 ? 2 : 1
    }

    /// The first code point of each of a row's `columns` cells, from that row's runs.
    static func codepoints(_ runs: [TerminalRun], columns: Int) -> [UInt32] {
        var row = [UInt32](repeating: 0, count: columns)
        for run in runs {
            var previous = -1
            var i = 0
            for scalar in run.text.unicodeScalars {
                let column = run.utf16Columns[i]
                if column != previous && row.indices.contains(column) { row[column] = scalar.value }
                previous = column
                i += UTF16.width(scalar)
            }
        }
        return row
    }
}
