#if canImport(UIKit)
import CoreText
import SwiftUI
import UIKit

/// Read-only terminal snapshot for SwiftUI. Input goes through the app's prompt bar and key strip.
public struct GhosttyTerminalView: UIViewRepresentable {
    public var snapshot: String
    public var fontSize: CGFloat

    public init(snapshot: String, fontSize: CGFloat = 12) {
        self.snapshot = snapshot
        self.fontSize = fontSize
    }

    public func makeUIView(context: Context) -> GhosttyTerminalUIView {
        GhosttyTerminalUIView(fontSize: fontSize)
    }

    public func updateUIView(_ view: GhosttyTerminalUIView, context: Context) {
        view.fontSize = fontSize
        view.show(ansiSnapshot: snapshot)
    }
}

/// Scrolls over the snapshot and draws only the visible cells, so a wide pane never needs a
/// backing store larger than the screen.
public final class GhosttyTerminalUIView: UIScrollView {
    static let background = TerminalRGB(0x28, 0x2C, 0x34)
    static let foreground = TerminalRGB(0xFF, 0xFF, 0xFF)

    public var fontSize: CGFloat {
        didSet {
            guard fontSize != oldValue else { return }
            canvas.metrics = CellMetrics(size: fontSize)
            wraps ? render() : prepare()
        }
    }

    /// Wraps rows at the view width instead of clipping them at the Mac pane width and
    /// scrolling horizontally.
    public var wraps = false {
        didSet {
            guard wraps != oldValue else { return }
            contentOffset.x = -adjustedContentInset.left
            render()
        }
    }

    private let screen = TerminalScreen(background: GhosttyTerminalUIView.background, foreground: GhosttyTerminalUIView.foreground)
    private let canvas = TerminalCanvas()
    private var snapshot: String?
    private var frameData: TerminalFrame?
    private var renderedColumns: Int?
    private var followsBottom = true
    private var laidOutSize = CGSize.zero

    public init(fontSize: CGFloat = 12) {
        self.fontSize = fontSize
        super.init(frame: .zero)
        backgroundColor = UIColor(Self.background)
        indicatorStyle = .white
        alwaysBounceVertical = true
        canvas.metrics = CellMetrics(size: fontSize)
        canvas.backgroundColor = UIColor(Self.background)
        canvas.isUserInteractionEnabled = false
        addSubview(canvas)
    }

    required init?(coder: NSCoder) {
        fatalError("init(coder:) is not supported")
    }

    public func show(ansiSnapshot: String) {
        guard ansiSnapshot != snapshot else { return }
        snapshot = ansiSnapshot
        render()
    }

    private var wrapColumns: Int? {
        let width = bounds.width - adjustedContentInset.left - adjustedContentInset.right - 2 * canvas.metrics.inset
        guard wraps, width > 0 else { return nil }
        return TerminalScreen.wrapColumns(width: width, cellWidth: canvas.metrics.width)
    }

    private func render() {
        guard let snapshot, let screen else { return }
        let columns = wrapColumns
        // Wait for a width: rendering at one column would flash a tall, narrow frame.
        if wraps && columns == nil { return }
        frameData = screen.render(ansiSnapshot: snapshot, wrapColumns: columns)
        renderedColumns = columns
        prepare()
    }

    private func prepare() {
        guard let frameData else { return }
        if bounds.height > 0 {
            followsBottom = contentOffset.y >= contentSize.height + adjustedContentInset.bottom - bounds.height - 1
        }
        canvas.prepare(frameData)
        let m = canvas.metrics
        contentSize = CGSize(
            width: CGFloat(frameData.columns) * m.width + 2 * m.inset,
            height: CGFloat(frameData.rows) * m.height + 2 * m.inset
        )
        if followsBottom { scrollToBottom() }
        setNeedsLayout()
        canvas.setNeedsDisplay()
    }

    public override func layoutSubviews() {
        super.layoutSubviews()
        if bounds.size != laidOutSize {
            laidOutSize = bounds.size
            if followsBottom { scrollToBottom() }
            if wraps && wrapColumns != renderedColumns { render() }
        }
        if canvas.frame != bounds {
            canvas.frame = bounds
            canvas.setNeedsDisplay()
        }
    }

    private func scrollToBottom() {
        contentOffset.y = max(contentSize.height + adjustedContentInset.bottom - bounds.height, -adjustedContentInset.top)
    }
}

/// MesloLGS NF, as the user's Ghostty uses: Nerd Font glyphs in prompts and status lines.
/// Loaded from the package bundle once, without registering it for other apps.
// CTFontDescriptor is immutable and documented thread-safe; CoreText does not mark it Sendable.
struct TerminalFont: @unchecked Sendable {
    let regular: CTFontDescriptor?
    let bold: CTFontDescriptor?
    let italic: CTFontDescriptor?
    let boldItalic: CTFontDescriptor?

    static let meslo = TerminalFont(
        regular: load("MesloLGS-NF-Regular"),
        bold: load("MesloLGS-NF-Bold"),
        italic: load("MesloLGS-NF-Italic"),
        boldItalic: load("MesloLGS-NF-BoldItalic")
    )

    private static func load(_ name: String) -> CTFontDescriptor? {
        guard let url = Bundle.module.url(forResource: name, withExtension: "ttf", subdirectory: "Fonts"),
            let data = try? Data(contentsOf: url)
        else { return nil }
        return CTFontManagerCreateFontDescriptorFromData(data as CFData)
    }
}

struct CellMetrics {
    let regular: CTFont
    let bold: CTFont
    let italic: CTFont
    let boldItalic: CTFont
    let width: CGFloat
    let height: CGFloat
    let ascent: CGFloat
    let inset: CGFloat = 8

    init(size: CGFloat) {
        let base = UIFont.monospacedSystemFont(ofSize: size, weight: .regular)
        let heavy = UIFont.monospacedSystemFont(ofSize: size, weight: .bold)
        func slanted(_ font: UIFont) -> UIFont {
            font.fontDescriptor.withSymbolicTraits(.traitItalic).map { UIFont(descriptor: $0, size: size) } ?? font
        }
        let meslo = TerminalFont.meslo
        func make(_ descriptor: CTFontDescriptor?, _ fallback: UIFont) -> CTFont {
            descriptor.map { CTFontCreateWithFontDescriptor($0, size, nil) } ?? fallback as CTFont
        }
        regular = make(meslo.regular, base)
        bold = make(meslo.bold, heavy)
        italic = make(meslo.italic, slanted(base))
        boldItalic = make(meslo.boldItalic, slanted(heavy))
        var glyph = CGGlyph()
        var advance = CGSize.zero
        var m: UniChar = 0x4D
        CTFontGetGlyphsForCharacters(regular, &m, &glyph, 1)
        CTFontGetAdvancesForGlyphs(regular, .horizontal, &glyph, &advance, 1)
        width = advance.width
        ascent = CTFontGetAscent(regular)
        height = ceil(ascent + CTFontGetDescent(regular) + CTFontGetLeading(regular))
    }

    func font(_ style: TerminalStyle) -> CTFont {
        switch (style.bold, style.italic) {
        case (false, false): regular
        case (true, false): bold
        case (false, true): italic
        case (true, true): boldItalic
        }
    }
}

private final class TerminalCanvas: UIView {
    private struct Prepared {
        let run: TerminalRun
        let line: CTLine?
    }

    var metrics = CellMetrics(size: 12)
    private var frameData: TerminalFrame?
    private var prepared: [Prepared] = []

    func prepare(_ frame: TerminalFrame) {
        frameData = frame
        prepared = frame.runs.map { run in
            guard !run.text.isEmpty, !run.style.invisible else { return Prepared(run: run, line: nil) }
            let attributed = NSAttributedString(
                string: run.text,
                attributes: [NSAttributedString.Key(kCTFontAttributeName as String): metrics.font(run.style)]
            )
            return Prepared(run: run, line: CTLineCreateWithAttributedString(attributed))
        }
    }

    override func draw(_ rect: CGRect) {
        guard let ctx = UIGraphicsGetCurrentContext(), let frameData else { return }
        let m = metrics
        let origin = frame.origin
        let firstRow = Int((origin.y - m.inset) / m.height) - 1
        let lastRow = Int((origin.y + bounds.height - m.inset) / m.height) + 1
        let firstColumn = Int((origin.x - m.inset) / m.width) - 1
        let lastColumn = Int((origin.x + bounds.width - m.inset) / m.width) + 1

        ctx.translateBy(x: m.inset - origin.x, y: m.inset - origin.y)
        let visible = prepared.filter {
            $0.run.row >= firstRow && $0.run.row <= lastRow
                && $0.run.endColumn > firstColumn && $0.run.startColumn <= lastColumn
        }

        for item in visible {
            guard let bg = item.run.style.background, bg != frameData.background else { continue }
            ctx.setFillColor(UIColor(bg).cgColor)
            ctx.fill(cellRect(item.run, m))
        }

        for item in visible {
            let run = item.run
            let color = UIColor(run.style.foreground).withAlphaComponent(run.style.faint ? 0.5 : 1).cgColor
            let baseline = CGFloat(run.row) * m.height + m.ascent
            if let line = item.line {
                ctx.saveGState()
                // CoreText draws y-up; flip around this row's baseline.
                ctx.translateBy(x: 0, y: baseline)
                ctx.scaleBy(x: 1, y: -1)
                ctx.textMatrix = .identity
                ctx.setFillColor(color)
                drawGlyphs(line, columns: run.utf16Columns, in: ctx, width: m.width)
                ctx.restoreGState()
            }
            if run.style.underline || run.style.strikethrough {
                let r = cellRect(run, m)
                let thickness = max(1, m.height / 16)
                ctx.setFillColor(color)
                if run.style.underline {
                    ctx.fill(CGRect(x: r.minX, y: baseline + thickness * 1.5, width: r.width, height: thickness))
                }
                if run.style.strikethrough {
                    ctx.fill(CGRect(x: r.minX, y: baseline - m.ascent * 0.35, width: r.width, height: thickness))
                }
            }
        }
    }

    private func cellRect(_ run: TerminalRun, _ m: CellMetrics) -> CGRect {
        CGRect(
            x: CGFloat(run.startColumn) * m.width,
            y: CGFloat(run.row) * m.height,
            width: CGFloat(run.endColumn - run.startColumn) * m.width,
            height: m.height
        )
    }

    /// Places every glyph at its cell's column, so fallback fonts (box drawing, emoji, CJK)
    /// cannot drift off the grid. Glyphs of one cluster keep their offsets from the first.
    private func drawGlyphs(_ line: CTLine, columns: [Int], in ctx: CGContext, width: CGFloat) {
        guard let glyphRuns = CTLineGetGlyphRuns(line) as? [CTRun] else { return }
        for glyphRun in glyphRuns {
            let count = CTRunGetGlyphCount(glyphRun)
            guard count > 0 else { continue }
            let attributes = CTRunGetAttributes(glyphRun) as NSDictionary
            guard let value = attributes[kCTFontAttributeName as String] else { continue }
            let font = value as! CTFont
            var glyphs = [CGGlyph](repeating: 0, count: count)
            var positions = [CGPoint](repeating: .zero, count: count)
            var indices = [CFIndex](repeating: 0, count: count)
            CTRunGetGlyphs(glyphRun, CFRange(), &glyphs)
            CTRunGetPositions(glyphRun, CFRange(), &positions)
            CTRunGetStringIndices(glyphRun, CFRange(), &indices)
            var column = -1
            var clusterStart: CGFloat = 0
            for i in 0..<count {
                let c = columns[min(max(indices[i], 0), columns.count - 1)]
                if c != column {
                    column = c
                    clusterStart = positions[i].x
                }
                positions[i] = CGPoint(x: CGFloat(c) * width + positions[i].x - clusterStart, y: positions[i].y)
            }
            CTFontDrawGlyphs(font, glyphs, positions, count, ctx)
        }
    }
}

extension UIColor {
    convenience init(_ c: TerminalRGB) {
        self.init(red: CGFloat(c.r) / 255, green: CGFloat(c.g) / 255, blue: CGFloat(c.b) / 255, alpha: 1)
    }
}
#endif
