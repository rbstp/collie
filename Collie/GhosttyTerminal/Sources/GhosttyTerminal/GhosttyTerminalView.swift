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
    private var selection: TerminalSelection? {
        didSet { selectionChanged() }
    }
    private var pressedWord: TerminalSelection?
    private var draggedEdge: TerminalSelection.Edge?
    private var dragOffset = CGPoint.zero
    private var loupe: UITextLoupeSession?
    private let startHandle = SelectionHandle(edge: .start)
    private let endHandle = SelectionHandle(edge: .end)
    private let copyButton = UIButton(configuration: .filled())
    private let openButton = UIButton(configuration: .filled())
    private let menu = UIStackView()
    /// Where the pop-up points, relative to the visible area so it stays put while the
    /// content decelerates.
    private var copyAnchor: CGPoint?
    private let press = UILongPressGestureRecognizer()
    private let tap = UITapGestureRecognizer()
    private let tapFilter = FlingTapFilter()

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

        press.minimumPressDuration = 0.45
        press.addTarget(self, action: #selector(pressed(_:)))
        addGestureRecognizer(press)
        tap.addTarget(self, action: #selector(tapped))
        tapFilter.scrollView = self
        tap.delegate = tapFilter
        addGestureRecognizer(tap)
        panGestureRecognizer.addTarget(self, action: #selector(panned(_:)))
        for handle in [startHandle, endHandle] {
            let drag = UILongPressGestureRecognizer(target: self, action: #selector(draggedHandle(_:)))
            // Begins on touch down, so it wins over the scroll view's pan and moves by one cell.
            drag.minimumPressDuration = 0
            drag.allowableMovement = .greatestFiniteMagnitude
            handle.addGestureRecognizer(drag)
            handle.isHidden = true
            addSubview(handle)
        }
        for button in [copyButton, openButton] {
            button.configuration?.cornerStyle = .capsule
            button.configuration?.contentInsets = NSDirectionalEdgeInsets(top: 6, leading: 14, bottom: 6, trailing: 14)
            button.configuration?.titleTextAttributesTransformer = UIConfigurationTextAttributesTransformer {
                var attributes = $0
                attributes.font = UIFont.preferredFont(forTextStyle: .subheadline)
                return attributes
            }
            menu.addArrangedSubview(button)
        }
        copyButton.accessibilityLabel = "Copy"
        copyButton.addAction(UIAction { [weak self] _ in self?.copySelection() }, for: .primaryActionTriggered)
        openButton.configuration?.title = "Open"
        openButton.addAction(UIAction { [weak self] _ in self?.openLink() }, for: .primaryActionTriggered)
        menu.spacing = 8
        menu.isHidden = true
        addSubview(menu)
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
        selection = selection?.clamped(to: frameData)
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
        if let copyAnchor, !menu.isHidden {
            let size = menu.systemLayoutSizeFitting(UIView.layoutFittingCompressedSize)
            let visible = bounds.inset(by: safeAreaInsets).insetBy(dx: 8, dy: 8)
            let point = CGPoint(x: bounds.minX + copyAnchor.x, y: bounds.minY + copyAnchor.y)
            var y = point.y - 16 - size.height
            if y < visible.minY { y = point.y + 16 }
            menu.frame = CGRect(
                x: min(max(point.x - size.width / 2, visible.minX), visible.maxX - size.width),
                y: min(max(y, visible.minY), visible.maxY - size.height),
                width: size.width,
                height: size.height
            )
        }
    }

    public override func gestureRecognizerShouldBegin(_ recognizer: UIGestureRecognizer) -> Bool {
        if recognizer === panGestureRecognizer || recognizer === press || recognizer === tap {
            if draggedEdge != nil { return false }
            if recognizer !== panGestureRecognizer && !menu.isHidden
                && menu.frame.contains(recognizer.location(in: self))
            {
                return false
            }
        }
        return super.gestureRecognizerShouldBegin(recognizer)
    }

    @objc private func pressed(_ recognizer: UILongPressGestureRecognizer) {
        let point = recognizer.location(in: self)
        switch recognizer.state {
        case .began:
            let m = canvas.metrics
            guard let frameData, point.y >= m.inset, point.y < m.inset + CGFloat(frameData.rows) * m.height,
                let cell = cell(at: point)
            else { return }
            let word = TerminalSelection.word(at: cell, in: frameData)
            hideCopy()
            pressedWord = word
            selection = word
            isScrollEnabled = false
            UIImpactFeedbackGenerator(style: .light, view: self).impactOccurred(at: point)
            loupe = UITextLoupeSession.begin(at: point, fromSelectionWidgetView: nil, in: self)
        case .changed:
            guard let pressedWord, let cell = cell(at: point) else { return }
            selection = pressedWord.extended(to: cell)
            loupe?.move(to: point, withCaretRect: .null, trackingCaret: false)
        default:
            let selecting = pressedWord != nil
            pressedWord = nil
            isScrollEnabled = true
            endLoupe()
            selectionChanged()
            if selecting && recognizer.state == .ended { showCopy(at: point) }
        }
    }

    @objc private func draggedHandle(_ recognizer: UILongPressGestureRecognizer) {
        let point = recognizer.location(in: self)
        switch recognizer.state {
        case .began:
            guard let handle = recognizer.view as? SelectionHandle, let selection else { return }
            draggedEdge = handle.edge
            let center = cellCenter(handle.edge == .start ? selection.start : selection.end)
            dragOffset = CGPoint(x: center.x - point.x, y: center.y - point.y)
            hideCopy()
            loupe = UITextLoupeSession.begin(at: point, fromSelectionWidgetView: handle, in: self)
        case .changed:
            guard let edge = draggedEdge, let selection,
                let cell = cell(at: CGPoint(x: point.x + dragOffset.x, y: point.y + dragOffset.y))
            else { return }
            let moved = selection.moving(edge, to: cell)
            draggedEdge = moved.edge
            if moved.selection != selection { self.selection = moved.selection }
            let handle = moved.edge == .start ? startHandle : endHandle
            loupe?.move(to: point, withCaretRect: handle.barFrame(in: self), trackingCaret: false)
        default:
            let dragging = draggedEdge != nil
            draggedEdge = nil
            endLoupe()
            if dragging && selection != nil { showCopy(at: point) }
        }
    }

    @objc private func panned(_ recognizer: UIPanGestureRecognizer) {
        switch recognizer.state {
        case .began:
            hideCopy()
        case .ended, .cancelled:
            if selection != nil && pressedWord == nil && draggedEdge == nil {
                showCopy(at: recognizer.location(in: self))
            }
        default:
            break
        }
    }

    @objc private func tapped() {
        if selection != nil { selection = nil }
    }

    private func cell(at point: CGPoint) -> TerminalCell? {
        guard let frameData else { return nil }
        let m = canvas.metrics
        return TerminalCell.at(
            x: point.x, y: point.y, cellWidth: m.width, cellHeight: m.height, inset: m.inset, in: frameData
        )
    }

    private func cellCenter(_ cell: TerminalCell) -> CGPoint {
        let m = canvas.metrics
        return CGPoint(x: m.inset + (CGFloat(cell.column) + 0.5) * m.width, y: m.inset + (CGFloat(cell.row) + 0.5) * m.height)
    }

    private func selectionChanged() {
        canvas.selection = selection
        canvas.setNeedsDisplay()
        guard let selection else {
            startHandle.isHidden = true
            endHandle.isHidden = true
            hideCopy()
            return
        }
        let m = canvas.metrics
        let content = CGRect(origin: .zero, size: contentSize)
        startHandle.place(
            x: m.inset + CGFloat(selection.start.column) * m.width,
            rowTop: m.inset + CGFloat(selection.start.row) * m.height,
            rowHeight: m.height,
            within: content
        )
        endHandle.place(
            x: m.inset + CGFloat(selection.end.column + 1) * m.width,
            rowTop: m.inset + CGFloat(selection.end.row) * m.height,
            rowHeight: m.height,
            within: content
        )
        startHandle.isHidden = pressedWord != nil
        endHandle.isHidden = pressedWord != nil
    }

    private func showCopy(at point: CGPoint) {
        copyAnchor = CGPoint(x: point.x - bounds.minX, y: point.y - bounds.minY)
        copyButton.configuration?.title = "Copy"
        copyButton.isUserInteractionEnabled = true
        openButton.isHidden = frameData.flatMap { selection?.link(in: $0) } == nil
        menu.isHidden = false
        setNeedsLayout()
    }

    private func hideCopy() {
        menu.isHidden = true
        copyAnchor = nil
    }

    private func endLoupe() {
        loupe?.invalidate()
        loupe = nil
    }

    private func copySelection() {
        guard let selection, let frameData else { return }
        // Universal Clipboard stays on so a selection can be pasted on the Mac.
        UIPasteboard.general.string = selection.text(in: frameData)
        UINotificationFeedbackGenerator(view: self).notificationOccurred(.success, at: menu.convert(copyButton.center, to: self))
        copyButton.configuration?.title = "Copied"
        copyButton.isUserInteractionEnabled = false
        setNeedsLayout()
        Task { [weak self] in
            try? await Task.sleep(for: .milliseconds(800))
            guard let self, self.selection == selection else { return }
            self.selection = nil
        }
    }

    private func openLink() {
        guard let frameData, let url = selection?.link(in: frameData) else { return }
        UIApplication.shared.open(url)
        selection = nil
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

/// A tap that only stops a fling must not clear the selection; touch-down is the last moment
/// `isDecelerating` still tells them apart.
@MainActor
private final class FlingTapFilter: NSObject, UIGestureRecognizerDelegate {
    weak var scrollView: UIScrollView?

    func gestureRecognizer(_ gestureRecognizer: UIGestureRecognizer, shouldReceive touch: UITouch) -> Bool {
        scrollView?.isDecelerating != true
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
    var selection: TerminalSelection?
    private var frameData: TerminalFrame?
    private var prepared: [Prepared] = []

    override func tintColorDidChange() {
        super.tintColorDidChange()
        setNeedsDisplay()
    }

    func prepare(_ frame: TerminalFrame) {
        frameData = frame
        prepared = frame.runs.map { run in
            guard !run.text.isEmpty, !run.style.invisible else { return Prepared(run: run, line: nil) }
            // Claude Code's ⏺ is in no bundled or system text font, so CoreText falls back to the color
            // emoji; ● is the same dot in Meslo and the same UTF-16 length, so `utf16Columns` still lines up.
            let attributed = NSAttributedString(
                string: run.text.replacingOccurrences(of: "\u{23FA}", with: "\u{25CF}"),
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

        if let selection {
            ctx.setFillColor(tintColor.withAlphaComponent(0.35).cgColor)
            for row in stride(from: max(selection.start.row, firstRow), through: min(selection.end.row, lastRow), by: 1) {
                guard let columns = selection.columns(inRow: row, columns: frameData.columns) else { continue }
                ctx.fill(
                    CGRect(
                        x: CGFloat(columns.lowerBound) * m.width,
                        y: CGFloat(row) * m.height,
                        width: CGFloat(columns.count) * m.width,
                        height: m.height
                    )
                )
            }
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

/// An iOS selection handle: a bar the height of the row with a dot above it at the start and
/// below it at the end. The view is the 44 pt hit area on the dot's side of the row.
private final class SelectionHandle: UIView {
    let edge: TerminalSelection.Edge
    private let bar = UIView()
    private let dot = UIView()

    init(edge: TerminalSelection.Edge) {
        self.edge = edge
        super.init(frame: .zero)
        dot.layer.cornerRadius = 5
        for part in [bar, dot] {
            part.isUserInteractionEnabled = false
            addSubview(part)
        }
        tintColorDidChange()
    }

    required init?(coder: NSCoder) {
        fatalError("init(coder:) is not supported")
    }

    override func tintColorDidChange() {
        super.tintColorDidChange()
        bar.backgroundColor = tintColor
        dot.backgroundColor = tintColor
    }

    /// Puts the bar at `x` in the superview, along the row starting at `rowTop`.
    func place(x: CGFloat, rowTop: CGFloat, rowHeight: CGFloat, within content: CGRect) {
        let size: CGFloat = 44
        let middle = rowTop + rowHeight / 2
        // A scroll view only hit-tests inside its bounds, which end at the content's edges when
        // scrolled to them: slide the hit area over the row rather than lose part of it there.
        frame = CGRect(
            x: max(content.minX, min(x - size / 2, content.maxX - size)),
            y: max(content.minY, min(edge == .start ? middle - size : middle, content.maxY - size)),
            width: size,
            height: size
        )
        let barX = x - frame.minX
        let barTop = rowTop - frame.minY
        bar.frame = CGRect(x: barX - 1, y: barTop, width: 2, height: rowHeight)
        dot.frame = CGRect(x: barX - 5, y: edge == .start ? barTop - 9 : barTop + rowHeight - 1, width: 10, height: 10)
    }

    /// Only the handle's own drag starts from a touch on it, like UISlider does for scroll views.
    override func gestureRecognizerShouldBegin(_ recognizer: UIGestureRecognizer) -> Bool {
        recognizer.view === self
    }

    func barFrame(in view: UIView) -> CGRect {
        convert(bar.frame, to: view)
    }
}

extension UIColor {
    convenience init(_ c: TerminalRGB) {
        self.init(red: CGFloat(c.r) / 255, green: CGFloat(c.g) / 255, blue: CGFloat(c.b) / 255, alpha: 1)
    }
}
#endif
