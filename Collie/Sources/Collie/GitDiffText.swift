import SwiftUI
import UIKit

struct GitDiffRow: Identifiable, Equatable {
    enum Kind { case context, added, removed, hunk, notice }

    let id: Int
    let kind: Kind
    let text: String
    var oldLine: Int?
    var newLine: Int?

    var marker: String {
        switch kind {
        case .added: "+"
        case .removed: "−"
        default: ""
        }
    }

    static func parse(_ patch: String) -> [Self] {
        var rows: [Self] = []
        var old: Int?
        var new: Int?
        for line in patch.components(separatedBy: "\n") {
            let parts = line.split(separator: " ")
            if parts.count >= 4, parts[0] == "@@", parts[3] == "@@",
               parts[1].hasPrefix("-"), parts[2].hasPrefix("+"),
               let oldStart = parts[1].dropFirst().split(separator: ",").first.flatMap({ Int($0) }),
               let newStart = parts[2].dropFirst().split(separator: ",").first.flatMap({ Int($0) }) {
                old = oldStart
                new = newStart
                rows.append(Self(id: rows.count, kind: .hunk, text: line))
            } else if let oldLine = old, let newLine = new, let prefix = line.first,
                      prefix == " " || prefix == "+" || prefix == "-" {
                let kind: Kind = prefix == "+" ? .added : prefix == "-" ? .removed : .context
                rows.append(Self(id: rows.count, kind: kind, text: String(line.dropFirst()),
                                 oldLine: kind == .added ? nil : oldLine, newLine: kind == .removed ? nil : newLine))
                if kind != .added { old = oldLine + 1 }
                if kind != .removed { new = newLine + 1 }
            } else if line.hasPrefix("\\ ") || line.hasPrefix("Binary ") || line.hasPrefix("New empty ") {
                rows.append(Self(id: rows.count, kind: .notice, text: line))
            }
        }
        return rows
    }
}

struct GitDiffAnchor: Equatable {
    let patch: String
    let row: Int
}

struct GitDiffBlock: Identifiable {
    let rows: [GitDiffRow]
    var id: Int { rows[0].id }
    var lastRow: Int { rows[rows.count - 1].id }
    var text: String { rows.map { $0.text + "\n" }.joined() }

    var ranges: [NSRange] {
        var offset = 0
        return rows.map { row in
            let length = (row.text as NSString).length + 1
            defer { offset += length }
            return NSRange(location: offset, length: length)
        }
    }

    func selectedRow(_ range: NSRange) -> Int? {
        guard !GitFeedback.selectedText(in: text, range: range).isEmpty else { return nil }
        return ranges.lastIndex { NSIntersectionRange($0, range).length > 0 }.map { rows[$0].id }
    }

    static func split(_ rows: [GitDiffRow], after anchors: Set<Int>) -> [Self] {
        var blocks: [Self] = []
        var start = 0
        for index in rows.indices where anchors.contains(rows[index].id) {
            blocks.append(Self(rows: Array(rows[start...index])))
            start = index + 1
        }
        if start < rows.count { blocks.append(Self(rows: Array(rows[start...]))) }
        return blocks
    }
}

struct GitDiffText: UIViewRepresentable {
    let block: GitDiffBlock
    let wraps: Bool
    let minimumWidth: CGFloat
    let digits: Int
    var feedback: ((String, Int) -> Void)?
    @Environment(\.dynamicTypeSize) private var dynamicTypeSize

    func makeUIView(context: Context) -> UITextView {
        let storage = NSTextStorage()
        let layout = GitDiffLayoutManager()
        let container = NSTextContainer(size: .zero)
        storage.addLayoutManager(layout)
        layout.addTextContainer(container)
        let view = UITextView(frame: .zero, textContainer: container)
        view.isEditable = false
        view.isSelectable = true
        view.isScrollEnabled = false
        view.backgroundColor = .clear
        view.textContainer.lineFragmentPadding = 0
        view.textContainer.widthTracksTextView = false
        view.delegate = context.coordinator
        return view
    }

    func updateUIView(_ view: UITextView, context: Context) {
        context.coordinator.block = block
        context.coordinator.feedback = feedback
        let font = UIFont.monospacedSystemFont(ofSize: UIFont.preferredFont(forTextStyle: .caption1, compatibleWith: view.traitCollection).pointSize, weight: .regular)
        let layout = view.layoutManager as! GitDiffLayoutManager
        let numberWidth = ceil(CGFloat(digits) * font.pointSize * 0.61) + 8
        let lineBreakMode: NSLineBreakMode = wraps ? .byCharWrapping : .byClipping
        guard layout.rows != block.rows || layout.font != font || layout.numberWidth != numberWidth
            || view.textContainer.lineBreakMode != lineBreakMode else { return }
        layout.rows = block.rows
        layout.ranges = block.ranges
        layout.font = font
        layout.numberWidth = numberWidth
        view.textContainerInset = UIEdgeInsets(top: 0, left: layout.gutterWidth, bottom: 0, right: 12)
        view.attributedText = NSAttributedString(string: block.text, attributes: [.font: font, .foregroundColor: UIColor.label])
        view.textContainer.lineBreakMode = lineBreakMode
        layout.invalidateDisplay(forCharacterRange: NSRange(location: 0, length: view.textStorage.length))
    }

    func sizeThatFits(_ proposal: ProposedViewSize, uiView: UITextView, context: Context) -> CGSize? {
        let insets = uiView.textContainerInset
        let width = wraps ? (proposal.width ?? minimumWidth) : max(minimumWidth, ceil(uiView.attributedText.size().width) + insets.left + insets.right)
        uiView.textContainer.size = CGSize(width: max(1, width - insets.left - insets.right), height: .greatestFiniteMagnitude)
        uiView.layoutManager.ensureLayout(for: uiView.textContainer)
        return CGSize(width: width, height: ceil(uiView.layoutManager.usedRect(for: uiView.textContainer).height))
    }

    func makeCoordinator() -> Coordinator { Coordinator(block: block, feedback: feedback) }

    final class Coordinator: NSObject, UITextViewDelegate {
        var block: GitDiffBlock
        var feedback: ((String, Int) -> Void)?

        init(block: GitDiffBlock, feedback: ((String, Int) -> Void)?) {
            self.block = block
            self.feedback = feedback
        }

        func textView(_ textView: UITextView, editMenuForTextIn range: NSRange, suggestedActions: [UIMenuElement]) -> UIMenu? {
            guard feedback != nil, let row = block.selectedRow(range) else { return nil }
            let text = GitFeedback.selectedText(in: block.text, range: range)
            let action = UIAction(title: "Feedback", image: UIImage(systemName: "text.bubble")) { [weak self] _ in
                self?.feedback?(text, row)
            }
            return UIMenu(children: [action] + suggestedActions)
        }
    }
}

final class GitDiffLayoutManager: NSLayoutManager {
    var rows: [GitDiffRow] = []
    var ranges: [NSRange] = []
    var font = UIFont.monospacedSystemFont(ofSize: 12, weight: .regular)
    var numberWidth: CGFloat = 32
    var gutterWidth: CGFloat { numberWidth * 2 + font.pointSize + 16 }

    override func drawBackground(forGlyphRange glyphsToShow: NSRange, at origin: CGPoint) {
        guard let container = textContainers.first else { return }
        for (row, range) in zip(rows, ranges) {
            let glyphs = glyphRange(forCharacterRange: range, actualCharacterRange: nil)
            guard NSIntersectionRange(glyphs, glyphsToShow).length > 0 else { continue }
            let first = lineFragmentRect(forGlyphAt: glyphs.location, effectiveRange: nil)
            let last = lineFragmentRect(forGlyphAt: NSMaxRange(glyphs) - 1, effectiveRange: nil)
            let rect = CGRect(x: 0, y: origin.y + first.minY, width: origin.x + container.size.width + 12, height: last.maxY - first.minY)
            let color: UIColor? = switch row.kind {
            case .added: .systemGreen
            case .removed: .systemRed
            case .hunk: .systemBlue
            default: nil
            }
            if let color {
                color.withAlphaComponent(0.14).setFill()
                UIRectFill(rect)
            }
            UIColor.secondaryLabel.withAlphaComponent(0.05).setFill()
            UIRectFillUsingBlendMode(CGRect(x: 0, y: rect.minY, width: gutterWidth - font.pointSize - 8, height: rect.height), .normal)
            let paragraph = NSMutableParagraphStyle()
            paragraph.alignment = .right
            let attributes: [NSAttributedString.Key: Any] = [.font: font, .foregroundColor: UIColor.secondaryLabel, .paragraphStyle: paragraph]
            for (column, number) in [row.oldLine, row.newLine].enumerated() {
                if let number {
                    (String(number) as NSString).draw(in: CGRect(x: CGFloat(column) * numberWidth, y: rect.minY, width: numberWidth - 4, height: first.height), withAttributes: attributes)
                }
            }
            (row.marker as NSString).draw(at: CGPoint(x: numberWidth * 2 + 4, y: rect.minY), withAttributes: [.font: font, .foregroundColor: UIColor.label])
        }
        super.drawBackground(forGlyphRange: glyphsToShow, at: origin)
    }
}
