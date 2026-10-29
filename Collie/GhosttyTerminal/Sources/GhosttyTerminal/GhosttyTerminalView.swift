#if canImport(UIKit)
import CoreText
import UIKit

/// Scrolls over the snapshot and draws only the visible cells, so a wide pane never needs a
/// backing store larger than the screen.
public final class GhosttyTerminalUIView: UIScrollView {
    static let background = TerminalRGB(0x28, 0x2C, 0x34)
    public static let backgroundColor = UIColor(background)
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
            gesturesChanged()
        }
    }

    /// Height at the top covered by an overlay; rows scroll out from under it and short content stays below it.
    public var obscuredTop: CGFloat = 0 {
        didSet {
            guard obscuredTop != oldValue else { return }
            setNeedsLayout()
        }
    }

    /// Height at the bottom covered by an overlay; the last rows scroll up from under it.
    public var obscuredBottom: CGFloat = 0 {
        didSet {
            guard obscuredBottom != oldValue else { return }
            contentInset.bottom = obscuredBottom
            if followsBottom { scrollToBottom() }
            setNeedsLayout()
        }
    }

    // Each gesture below is off while a selection is active, and while its callback is nil.
    public var onDoubleTap: (() -> Void)? {
        didSet { gesturesChanged() }
    }
    public var onTripleTap: (() -> Void)? {
        didSet { gesturesChanged() }
    }
    public var onPinch: ((CGFloat) -> Void)? {
        didSet { gesturesChanged() }
    }
    public var onPinchEnded: (() -> Void)?
    /// Also off unless rows wrap: without wrapping, a sideways pan scrolls the terminal.
    public var onSwipe: ((TerminalSwipe) -> Void)? {
        didSet { gesturesChanged() }
    }
    public var canSwipe: ((TerminalSwipe) -> Bool)?
    /// Shows Quote in the selection menu, which hands this the selected text.
    public var onQuote: ((String) -> Void)?
    /// Shows Open for selected text this maps to a file path, which `onOpenFile` receives.
    public var filePath: ((String) -> String?)?
    public var onOpenFile: ((String) -> Void)?

    private let screen = TerminalScreen(background: GhosttyTerminalUIView.background, foreground: GhosttyTerminalUIView.foreground)
    private let canvas = TerminalCanvas()
    private var snapshot: String?
    private var renderDeferred = false
    private let scrollEnd = ScrollEnd()
    private(set) var frameData: TerminalFrame?
    private var renderedColumns: Int?
    private var followsBottom = true
    private var laidOutSize = CGSize.zero
    var selection: TerminalSelection? {
        didSet { selectionChanged() }
    }
    private var pressedWord: TerminalSelection?
    private var draggedEdge: TerminalSelection.Edge?
    private var dragOffset = CGPoint.zero
    private var loupe: UITextLoupeSession?
    private let startHandle = SelectionHandle(edge: .start)
    private let endHandle = SelectionHandle(edge: .end)
    private let copyButton = UIButton(configuration: .glass())
    let quoteButton = UIButton(configuration: .glass())
    private let openButton = UIButton(configuration: .glass())
    private let menu = UIStackView()
    /// Where the pop-up points, relative to the visible area so it stays put while the
    /// content decelerates.
    private var copyAnchor: CGPoint?
    private var shownLink: URL?
    private var shownPath: String?
    private let press = UILongPressGestureRecognizer()
    private let tap = UITapGestureRecognizer()
    private let doubleTap = UITapGestureRecognizer()
    private let tripleTap = UITapGestureRecognizer()
    private let tapFilter = FlingTapFilter()
    private let pinch = UIPinchGestureRecognizer()
    private let pinchFilter = PinchAlongsidePan()
    private var pinchStart: CGFloat = 0
    private let swipe = UIPanGestureRecognizer()
    private let swipeFilter = SwipeAlongsideScroll()

    public init(fontSize: CGFloat) {
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
        doubleTap.numberOfTapsRequired = 2
        doubleTap.addTarget(self, action: #selector(doubleTapped))
        tripleTap.numberOfTapsRequired = 3
        tripleTap.addTarget(self, action: #selector(tripleTapped))
        for multiTap in [doubleTap, tripleTap] {
            multiTap.delegate = tapFilter
            addGestureRecognizer(multiTap)
        }
        tap.require(toFail: doubleTap)
        tap.require(toFail: tripleTap)
        doubleTap.require(toFail: tripleTap)
        pinch.addTarget(self, action: #selector(pinched(_:)))
        pinch.delegate = pinchFilter
        addGestureRecognizer(pinch)
        swipe.maximumNumberOfTouches = 1
        swipe.addTarget(self, action: #selector(swiped(_:)))
        swipeFilter.terminal = self
        swipe.delegate = swipeFilter
        addGestureRecognizer(swipe)
        gesturesChanged()
        panGestureRecognizer.addTarget(self, action: #selector(panned(_:)))
        scrollEnd.ended = { [weak self] in
            guard let self else { return }
            followsBottom = atBottom
            if renderDeferred { render() }
        }
        delegate = scrollEnd
        for handle in [startHandle, endHandle] {
            let drag = UILongPressGestureRecognizer(target: self, action: #selector(draggedHandle(_:)))
            // Begins on touch down, so it wins over the scroll view's pan and moves by one cell.
            drag.minimumPressDuration = 0
            drag.allowableMovement = .greatestFiniteMagnitude
            handle.addGestureRecognizer(drag)
            handle.isHidden = true
            addSubview(handle)
        }
        for button in [copyButton, quoteButton, openButton] {
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
        quoteButton.configuration?.title = "Quote"
        quoteButton.addAction(UIAction { [weak self] _ in self?.quoteSelection() }, for: .primaryActionTriggered)
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
        // A full history takes over 10 ms to render on the main thread, which would stutter
        // a scroll in progress.
        if isDragging || isDecelerating {
            renderDeferred = true
            return
        }
        render()
    }

    private var wrapColumns: Int? {
        let width = bounds.width - adjustedContentInset.left - adjustedContentInset.right - 2 * canvas.metrics.inset
        guard wraps, width > 0 else { return nil }
        return TerminalScreen.wrapColumns(width: width, cellWidth: canvas.metrics.width)
    }

    private func render() {
        guard let snapshot, let screen else { return }
        renderDeferred = false
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
            followsBottom = atBottom
        }
        canvas.prepare(frameData)
        let m = canvas.metrics
        contentSize = CGSize(
            width: CGFloat(frameData.columns) * m.width + 2 * m.inset,
            height: CGFloat(frameData.rows) * m.height + 2 * m.inset
        )
        selection = selection?.clamped(to: frameData)
        pinShortContent()
        if followsBottom { scrollToBottom() }
        setNeedsLayout()
        canvas.setNeedsDisplay()
    }

    public override func layoutSubviews() {
        super.layoutSubviews()
        if pinShortContent() || bounds.size != laidOutSize {
            laidOutSize = bounds.size
            if followsBottom { scrollToBottom() }
            if wraps && wrapColumns != renderedColumns {
                let (old, height, offset, follows) = (frameData, contentSize.height, contentOffset.y, followsBottom)
                render()
                // render() measures the bottom against the old content in the new bounds.
                followsBottom = follows
                if follows {
                    scrollToBottom()
                } else if let old, height > 0 {
                    keepTopLine(of: old, height: height, offset: offset)
                }
            }
        }
        if canvas.frame != bounds {
            canvas.frame = bounds
            canvas.setNeedsDisplay()
        }
        if let copyAnchor, !menu.isHidden {
            let size = menu.systemLayoutSizeFitting(UIView.layoutFittingCompressedSize)
            var insets = safeAreaInsets
            insets.top = max(insets.top, obscuredTop)
            insets.bottom = max(insets.bottom, obscuredBottom)
            let visible = bounds.inset(by: insets).insetBy(dx: 8, dy: 8)
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
            let extended = pressedWord.extended(to: cell)
            if extended != selection { selection = extended }
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

    @objc private func doubleTapped() {
        onDoubleTap?()
    }

    @objc private func tripleTapped() {
        onTripleTap?()
    }

    @objc private func pinched(_ recognizer: UIPinchGestureRecognizer) {
        switch recognizer.state {
        case .began:
            pinchStart = fontSize
            isScrollEnabled = false
            swipe.isEnabled = false
        case .changed:
            let size = TerminalFontSize.pinched(pinchStart, scale: recognizer.scale)
            guard size != fontSize else { return }
            let point = recognizer.location(in: self)
            let (old, offset) = (contentSize, contentOffset)
            fontSize = size
            if old.width > 0 && old.height > 0 {
                let inset = adjustedContentInset
                contentOffset.x = TerminalFontSize.pinchedOffset(
                    point.x, offset: offset.x, from: old.width, to: contentSize.width,
                    viewport: bounds.width, leading: inset.left, trailing: inset.right
                )
                if !followsBottom {
                    contentOffset.y = TerminalFontSize.pinchedOffset(
                        point.y, offset: offset.y, from: old.height, to: contentSize.height,
                        viewport: bounds.height, leading: inset.top, trailing: inset.bottom
                    )
                }
            }
            onPinch?(size)
        default:
            isScrollEnabled = true
            gesturesChanged()
            onPinchEnded?()
        }
    }

    @objc private func swiped(_ recognizer: UIPanGestureRecognizer) {
        // Measured outside the scroll view: inside, content scrolling under the finger hides vertical travel.
        guard recognizer.state == .ended,
            let swipe = TerminalSwipe.ended(translation: recognizer.translation(in: superview), velocity: recognizer.velocity(in: superview))
        else { return }
        onSwipe?(swipe)
    }

    private func gesturesChanged() {
        doubleTap.isEnabled = onDoubleTap != nil && selection == nil
        tripleTap.isEnabled = onTripleTap != nil && selection == nil
        pinch.isEnabled = onPinch != nil && selection == nil
        swipe.isEnabled = onSwipe != nil && selection == nil && wraps
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
        gesturesChanged()
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
        copyButton.configuration?.image = nil
        copyButton.configuration?.baseForegroundColor = nil
        copyButton.accessibilityLabel = "Copy"
        copyButton.isUserInteractionEnabled = true
        shownLink = frameData.flatMap { selection?.link(in: $0) }
        shownPath = shownLink == nil ? selectedPath() : nil
        quoteButton.isHidden = onQuote == nil
        openButton.isHidden = shownLink == nil && shownPath == nil
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
        UIView.transition(with: copyButton, duration: 0.2, options: .transitionCrossDissolve) {
            self.copyButton.configuration?.title = "Copied"
            self.copyButton.configuration?.image = UIImage(systemName: "checkmark.circle.fill")
            self.copyButton.configuration?.baseForegroundColor = .systemGreen
        }
        copyButton.accessibilityLabel = "Copied"
        copyButton.isUserInteractionEnabled = false
        setNeedsLayout()
        Task { [weak self] in
            try? await Task.sleep(for: .milliseconds(800))
            guard let self, self.selection == selection else { return }
            self.selection = nil
        }
    }

    private func quoteSelection() {
        if let selection, let frameData { onQuote?(selection.text(in: frameData)) }
        selection = nil
    }

    private func openLink() {
        // Frames keep arriving under an open menu; open only the link it was shown for.
        if let frameData, let shownLink, selection?.link(in: frameData) == shownLink {
            UIApplication.shared.open(shownLink)
        } else if let shownPath, selectedPath() == shownPath {
            onOpenFile?(shownPath)
        }
        selection = nil
    }

    private func selectedPath() -> String? {
        guard let filePath, onOpenFile != nil, let selection, let frameData else { return nil }
        return filePath(selection.text(in: frameData))
    }

    @discardableResult
    private func pinShortContent() -> Bool {
        guard refreshControl?.isRefreshing != true else { return false }
        let others = adjustedContentInset.top - contentInset.top + adjustedContentInset.bottom
        let gap = max(obscuredTop, bounds.height - others - contentSize.height)
        guard abs(gap - contentInset.top) > 0.5 else { return false }
        contentInset.top = gap
        return true
    }

    /// Keeps the line at the top of the viewport there across a rewrap, or its proportional place when soft wraps are unknown.
    private func keepTopLine(of old: TerminalFrame, height: CGFloat, offset: CGFloat) {
        guard let new = frameData else { return }
        let (m, inset) = (canvas.metrics, adjustedContentInset)
        guard !old.wrapsUnknown && !new.wrapsUnknown else {
            contentOffset.y = TerminalFontSize.pinchedOffset(
                offset + inset.top, offset: offset, from: height, to: contentSize.height,
                viewport: bounds.height, leading: inset.top, trailing: inset.bottom
            )
            return
        }
        let top = min(max(Int((offset + inset.top - m.inset) / m.height), 0), max(old.rows - 1, 0))
        let line = (0...top).count { !old.wrapContinuations.contains($0) } - 1
        let starts = (0..<new.rows).filter { !new.wrapContinuations.contains($0) }
        guard !starts.isEmpty else { return }
        let row = starts[min(max(line, 0), starts.count - 1)]
        let bottom = max(contentSize.height + inset.bottom - bounds.height, -inset.top)
        contentOffset.y = min(max(offset + CGFloat(row - top) * m.height, -inset.top), bottom)
    }

    private var atBottom: Bool {
        contentOffset.y >= contentSize.height + adjustedContentInset.bottom - bounds.height - 1
    }

    private func scrollToBottom() {
        contentOffset.y = max(contentSize.height + adjustedContentInset.bottom - bounds.height, -adjustedContentInset.top)
    }
}

/// Ghostty's default font: JetBrains Mono at weight 400 and 700, with Symbols Nerd Font first in
/// the cascade for prompt and status-line glyphs. Loaded from the package bundle once, without
/// registering it for other apps.
// CTFontDescriptor is immutable and documented thread-safe; CoreText does not mark it Sendable.
struct TerminalFont: @unchecked Sendable {
    let regular: CTFontDescriptor?
    let bold: CTFontDescriptor?
    let italic: CTFontDescriptor?
    let boldItalic: CTFontDescriptor?

    static let jetBrainsMono: TerminalFont = {
        let symbols = load("SymbolsNerdFont-Regular")
        func face(_ name: String, weight: CGFloat) -> CTFontDescriptor? {
            load(name).map {
                CTFontDescriptorCreateCopyWithAttributes(
                    CTFontDescriptorCreateCopyWithVariation($0, 0x7767_6874 as CFNumber, weight),  // 'wght'
                    [kCTFontCascadeListAttribute: symbols.map { [$0] } ?? []] as CFDictionary
                )
            }
        }
        return TerminalFont(
            regular: face("JetBrainsMono[wght]", weight: 400),
            bold: face("JetBrainsMono[wght]", weight: 700),
            italic: face("JetBrainsMono-Italic[wght]", weight: 400),
            boldItalic: face("JetBrainsMono-Italic[wght]", weight: 700)
        )
    }()

    private static func load(_ name: String) -> CTFontDescriptor? {
        guard let url = Bundle.module.url(forResource: name, withExtension: "ttf", subdirectory: "Fonts"),
            let data = try? Data(contentsOf: url)
        else { return nil }
        return CTFontManagerCreateFontDescriptorFromData(data as CFData)
    }
}

@MainActor
private final class ScrollEnd: NSObject, UIScrollViewDelegate {
    var ended: (() -> Void)?

    func scrollViewDidEndDragging(_ scrollView: UIScrollView, willDecelerate decelerate: Bool) {
        if !decelerate { ended?() }
    }

    func scrollViewDidEndDecelerating(_ scrollView: UIScrollView) {
        ended?()
    }

    func scrollViewDidScrollToTop(_ scrollView: UIScrollView) {
        ended?()
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

/// A swipe tracks alongside scrolling rather than holding it off, and leaves to the back swipe
/// touches that start at the left screen edge and drags toward a side `canSwipe` refuses.
@MainActor
private final class SwipeAlongsideScroll: NSObject, UIGestureRecognizerDelegate {
    weak var terminal: GhosttyTerminalUIView?

    func gestureRecognizer(_ gestureRecognizer: UIGestureRecognizer, shouldReceive touch: UITouch) -> Bool {
        touch.location(in: nil).x > 24
    }

    func gestureRecognizerShouldBegin(_ gestureRecognizer: UIGestureRecognizer) -> Bool {
        guard let pan = gestureRecognizer as? UIPanGestureRecognizer, let terminal, let canSwipe = terminal.canSwipe
        else { return true }
        return canSwipe(pan.translation(in: terminal.superview).x < 0 ? .left : .right)
    }

    func gestureRecognizer(
        _ gestureRecognizer: UIGestureRecognizer, shouldRecognizeSimultaneouslyWith other: UIGestureRecognizer
    ) -> Bool {
        other === terminal?.panGestureRecognizer
    }
}

/// Two fingers that already started scrolling can still pinch.
@MainActor
private final class PinchAlongsidePan: NSObject, UIGestureRecognizerDelegate {
    func gestureRecognizer(
        _ gestureRecognizer: UIGestureRecognizer, shouldRecognizeSimultaneouslyWith other: UIGestureRecognizer
    ) -> Bool {
        other is UIPanGestureRecognizer
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
    let descent: CGFloat
    let capHeight: CGFloat
    let inset: CGFloat = 8

    init(size: CGFloat) {
        let base = UIFont.monospacedSystemFont(ofSize: size, weight: .regular)
        let heavy = UIFont.monospacedSystemFont(ofSize: size, weight: .bold)
        func slanted(_ font: UIFont) -> UIFont {
            font.fontDescriptor.withSymbolicTraits(.traitItalic).map { UIFont(descriptor: $0, size: size) } ?? font
        }
        let font = TerminalFont.jetBrainsMono
        func make(_ descriptor: CTFontDescriptor?, _ fallback: UIFont) -> CTFont {
            descriptor.map { CTFontCreateWithFontDescriptor($0, size, nil) } ?? fallback as CTFont
        }
        regular = make(font.regular, base)
        bold = make(font.bold, heavy)
        italic = make(font.italic, slanted(base))
        boldItalic = make(font.boldItalic, slanted(heavy))
        var glyph = CGGlyph()
        var advance = CGSize.zero
        var m: UniChar = 0x4D
        CTFontGetGlyphsForCharacters(regular, &m, &glyph, 1)
        CTFontGetAdvancesForGlyphs(regular, .horizontal, &glyph, &advance, 1)
        width = advance.width
        ascent = CTFontGetAscent(regular)
        descent = CTFontGetDescent(regular)
        capHeight = CTFontGetCapHeight(regular)
        height = ceil(ascent + descent + CTFontGetLeading(regular))
    }

    func font(_ style: TerminalStyle) -> CTFont {
        switch (style.bold, style.italic) {
        case (false, false): regular
        case (true, false): bold
        case (false, true): italic
        case (true, true): boldItalic
        }
    }

    /// Maps a glyph with `bounds`, drawn from `font` with its origin at a cell's left edge on the
    /// baseline (y up), to where Ghostty would draw it, or nil to leave it where the font puts it.
    /// `cells` is `GhosttyCell.constraintWidth` for its cell.
    func placement(
        of scalar: UInt32, font: CTFont, bounds: @autoclosure () -> CGRect, cells: @autoclosure () -> Int
    ) -> CGAffineTransform? {
        if (0x2580...0x259F).contains(scalar) {
            let sy = height / (ascent + descent)
            return CGAffineTransform(a: 1, b: 0, c: 0, d: sy, tx: 0, ty: descent * sy + ascent - height)
        }
        // Only our fonts' glyphs: a PUA glyph from a system fallback font is no Nerd Font glyph.
        guard let constraint = NerdConstraint.lookup(scalar),
            ["JetBrains Mono", "Symbols Nerd Font"].contains(CTFontCopyFamilyName(font) as String)
        else { return nil }
        let b = bounds()
        guard b.width > 0, b.height > 0 else { return nil }
        // y from the cell's bottom, where Ghostty measures it; the baseline sits `ascent` below the top.
        let bottom = ascent - height
        let r: CGRect
        switch scalar {
        case 0xE0B0...0xE0BF, 0xE0D2, 0xE0D4:
            // Ghostty draws these as sprites filling the cell (`src/font/sprite/draw/powerline.zig`).
            r = CGRect(x: 0, y: bottom, width: width, height: height)
        default:
            let face = ascent + descent
            let metrics = NerdConstraint.Metrics(
                cellWidth: width, cellHeight: height, faceWidth: width, faceHeight: face, faceY: height - face,
                iconHeight: face, iconHeightSingle: (2 * capHeight + face) / 3
            )
            r = constraint.constrain(b.offsetBy(dx: 0, dy: -bottom), metrics, constraintWidth: cells())
                .offsetBy(dx: 0, dy: bottom)
        }
        let sx = r.width / b.width
        let sy = r.height / b.height
        return CGAffineTransform(a: sx, b: 0, c: 0, d: sy, tx: r.minX - b.minX * sx, ty: r.minY - b.minY * sy)
    }
}

private final class TerminalCanvas: UIView {
    var metrics = CellMetrics(size: 12) {
        didSet {
            lines = [:]
            previousLines = [:]
        }
    }
    var selection: TerminalSelection?
    private var frameData: TerminalFrame?
    /// Built on first draw: most runs of a long history are never on screen.
    private var lines: [LineKey: (line: CTLine, utf16: [UInt16])] = [:]
    private var previousLines: [LineKey: (line: CTLine, utf16: [UInt16])] = [:]

    private struct LineKey: Hashable {
        let text: String
        let bold: Bool
        let italic: Bool
    }

    override func tintColorDidChange() {
        super.tintColorDidChange()
        setNeedsDisplay()
    }

    func prepare(_ frame: TerminalFrame) {
        frameData = frame
        previousLines = lines
        lines = [:]
    }

    private func ctLine(_ run: TerminalRun) -> (line: CTLine, utf16: [UInt16])? {
        guard !run.text.isEmpty, !run.style.invisible else { return nil }
        let key = LineKey(text: run.text, bold: run.style.bold, italic: run.style.italic)
        if let line = lines[key] ?? previousLines[key] {
            lines[key] = line
            return line
        }
        // Claude Code's ⏺ is in no bundled or system text font, so CoreText falls back to the color
        // emoji; ● is the same dot in JetBrains Mono and the same UTF-16 length, so `utf16Columns` still lines up.
        let attributed = NSAttributedString(
            string: run.text.replacingOccurrences(of: "\u{23FA}", with: "\u{25CF}"),
            attributes: [NSAttributedString.Key(kCTFontAttributeName as String): metrics.font(run.style)]
        )
        let line = (CTLineCreateWithAttributedString(attributed), Array(run.text.utf16))
        lines[key] = line
        return line
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
        let visible = frameData.runs.indices.filter {
            let run = frameData.runs[$0]
            return run.row >= firstRow && run.row <= lastRow && run.endColumn > firstColumn && run.startColumn <= lastColumn
        }

        for index in visible {
            let run = frameData.runs[index]
            guard let bg = run.style.background, bg != frameData.background else { continue }
            ctx.setFillColor(UIColor(bg).cgColor)
            ctx.fill(cellRect(run, m))
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

        var rowCodepoints: [Int: [UInt32]] = [:]
        for index in visible {
            let run = frameData.runs[index]
            let color = UIColor(run.style.foreground).withAlphaComponent(run.style.faint ? 0.5 : 1).cgColor
            let baseline = CGFloat(run.row) * m.height + m.ascent
            if let (line, utf16) = ctLine(run) {
                ctx.saveGState()
                // CoreText draws y-up; flip around this row's baseline.
                ctx.translateBy(x: 0, y: baseline)
                ctx.scaleBy(x: 1, y: -1)
                ctx.textMatrix = .identity
                ctx.setFillColor(color)
                drawGlyphs(line, text: utf16, columns: run.utf16Columns, in: ctx, metrics: m) { column in
                    if rowCodepoints[run.row] == nil {
                        let runs = frameData.runs.filter { $0.row == run.row }
                        rowCodepoints[run.row] = GhosttyCell.codepoints(runs, columns: frameData.columns + 1)
                    }
                    return GhosttyCell.constraintWidth(rowCodepoints[run.row]!, column)
                }
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
    private func drawGlyphs(
        _ line: CTLine, text: [UInt16], columns: [Int], in ctx: CGContext, metrics m: CellMetrics,
        constraintWidth: (Int) -> Int
    ) {
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
            var plain = 0
            for i in 0..<count {
                let index = min(max(indices[i], 0), columns.count - 1)
                let c = columns[index]
                if c != column {
                    column = c
                    clusterStart = positions[i].x
                }
                if index < text.count, text[index] >= 0x80 {
                    let value = UTF16.isLeadSurrogate(text[index]) && index + 1 < text.count
                        ? 0x10000 + (UInt32(text[index]) - 0xD800) << 10 + UInt32(text[index + 1]) - 0xDC00
                        : UInt32(text[index])
                    var glyph = glyphs[i]
                    let bounds = { CTFontGetBoundingRectsForGlyphs(font, .horizontal, &glyph, nil, 1) }
                    if let placement = m.placement(of: value, font: font, bounds: bounds(), cells: constraintWidth(c)) {
                        ctx.saveGState()
                        ctx.translateBy(x: CGFloat(c) * m.width, y: 0)
                        ctx.concatenate(placement)
                        // A row edge can fall inside a pixel, where two antialiased solid blocks would leave
                        // a seam. Shades (U+2591 to U+2593) keep antialiasing, which their dot patterns need.
                        if (0x2580...0x2590).contains(value) || (0x2594...0x259F).contains(value) {
                            ctx.setShouldAntialias(false)
                        }
                        var origin = CGPoint.zero
                        CTFontDrawGlyphs(font, &glyph, &origin, 1, ctx)
                        ctx.restoreGState()
                        continue
                    }
                }
                glyphs[plain] = glyphs[i]
                positions[plain] = CGPoint(x: CGFloat(c) * m.width + positions[i].x - clusterStart, y: positions[i].y)
                plain += 1
            }
            CTFontDrawGlyphs(font, glyphs, positions, plain, ctx)
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
