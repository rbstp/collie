import CollieCore
import SwiftUI
import XCTest
@testable import Collie

@MainActor
final class GitDiffViewTests: XCTestCase {
    func testSelectionInsertsAnInlineEditorAcrossAppearanceAndWrapping() async throws {
        let savedWraps = UserDefaults.standard.object(forKey: "gitWrapLines")
        defer { UserDefaults.standard.set(savedWraps, forKey: "gitWrapLines") }
        for (name, style, size, wraps) in [
            ("light", UIUserInterfaceStyle.light, UIContentSizeCategory.large, true),
            ("dark", .dark, .large, true),
            ("large-text", .dark, .accessibilityExtraExtraExtraLarge, true),
            ("no-wrap", .light, .large, false)
        ] {
            UserDefaults.standard.set(wraps, forKey: "gitWrapLines")
            let core = FakeCore()
            core.state.withLock {
                $0.gitPatch = "diff --git a/file.swift b/file.swift\nindex 123..456 100644\n--- a/file.swift\n+++ b/file.swift\n@@ -43,4 +43,4 @@ struct UsageSection\n struct UsageSection: View {\n-    let title: String\n+    let period: String\n     var body: some View {\n         Text(\"A long line with 🐑 and enough text to wrap across the entire width of the phone screen\")\n"
                if name == "light" {
                    $0.gitDiffDelay = .milliseconds(600)
                    $0.gitPatch = $0.gitPatch.replacingOccurrences(of: " struct UsageSection: View {", with: String(repeating: " context\n", count: 20) + " struct UsageSection: View {")
                }
            }
            let model = AgentModel(core: core, route: AgentRoute(machineId: "test", terminalId: "test"), prefsFile: nil)
            let review = GitReview()
            let file = GitFile(path: "Collie/UsageSection.swift", oldPath: nil, section: "unstaged", status: "M", additions: 1, deletions: 1)
            let host = UIHostingController(rootView: NavigationStack {
                GitDiffView(agent: model, root: "/worktree", file: file, sent: {}, review: review)
            })
            host.overrideUserInterfaceStyle = style
            host.traitOverrides.preferredContentSizeCategory = size
            try await hosting(host) { window in
                let textView = try await waitFor("\(name) diff view") {
                    host.view.layoutIfNeeded()
                    return descendants(host.view).compactMap { $0 as? UITextView }.first {
                        $0.text.contains("period") && !$0.bounds.isEmpty
                    }
                }
                XCTAssertFalse(textView.text.contains("diff --git"))
                XCTAssertFalse(textView.text.contains("+    let"))
                XCTAssertGreaterThan(textView.textContainerInset.left, 40)
                if size.isAccessibilityCategory { XCTAssertGreaterThan(textView.font?.pointSize ?? 0, 20) }
                if !wraps { XCTAssertGreaterThan(textView.bounds.width, window.bounds.width) }
                let longLine = (textView.text as NSString).range(of: "Text(\"A long line with 🐑 and enough text to wrap across the entire width of the phone screen\")")
                let glyphs = textView.layoutManager.glyphRange(forCharacterRange: longLine, actualCharacterRange: nil)
                let firstLine = textView.layoutManager.lineFragmentRect(forGlyphAt: glyphs.location, effectiveRange: nil)
                let lastLine = textView.layoutManager.lineFragmentRect(forGlyphAt: NSMaxRange(glyphs) - 1, effectiveRange: nil)
                XCTAssertEqual(lastLine.minY > firstLine.minY, wraps)
                XCTAssertGreaterThanOrEqual(textView.bounds.height, lastLine.maxY)
                let selection = (textView.text as NSString).range(of: "period")
                textView.selectedRange = selection
                let menu = try XCTUnwrap(textView.delegate?.textView?(textView, editMenuForTextIn: selection, suggestedActions: []))
                let action = try XCTUnwrap(menu.children.first as? UIAction)
                let button = UIButton(primaryAction: action)
                button.sendActions(for: .touchUpInside)
                let views = try await waitFor("\(name) inline editor") {
                    host.view.layoutIfNeeded()
                    let views = descendants(host.view).compactMap { $0 as? UITextView }.filter { !$0.isEditable && $0.layoutManager is GitDiffLayoutManager }
                    return views.count == 2 && views.allSatisfy { !$0.bounds.isEmpty } ? views : nil
                }
                XCTAssertEqual(review.comments.count, 1)
                XCTAssertEqual(review.comments.first?.selection, "period")
                XCTAssertEqual(review.comments.first?.anchor?.row, name == "light" ? 23 : 3)
                XCTAssertEqual(views.count, 2)
                let before = try XCTUnwrap(views.first { $0.text.contains("period") })
                let after = try XCTUnwrap(views.first { $0.text.contains("var body") })
                XCTAssertGreaterThan(after.convert(after.bounds, to: host.view).minY - before.convert(before.bounds, to: host.view).maxY, 40)
                if name == "light" {
                    for inset in [CGFloat(240), 320] {
                        host.additionalSafeAreaInsets.bottom = inset
                        _ = try await waitFor("focused editor above the \(Int(inset)) inset") {
                            host.view.layoutIfNeeded()
                            guard let field = descendants(host.view).compactMap({ $0 as? UITextView }).first(where: { $0.isEditable && $0.isFirstResponder }) else { return nil as Bool? }
                            var ancestor = field.superview
                            while ancestor != nil, !(ancestor is UIScrollView) { ancestor = ancestor?.superview }
                            guard let viewport = ancestor as? UIScrollView else { return nil }
                            let visible = viewport.bounds.inset(by: viewport.adjustedContentInset)
                            let frame = field.convert(field.bounds, to: viewport)
                            return !frame.isEmpty && frame.minY >= visible.minY - 1 && frame.maxY <= visible.maxY + 1 ? true : nil
                        }
                        snapshot(window, name: "keyboard-\(Int(inset))")
                    }
                    host.additionalSafeAreaInsets.bottom = 0
                }
                host.view.endEditing(true)
                _ = try await waitFor("feedback to lose focus") {
                    host.view.layoutIfNeeded()
                    return descendants(host.view).contains(where: \.isFirstResponder) ? nil : true
                }
                snapshot(window, name: name)
            }
        }
    }

    private func snapshot(_ window: UIWindow, name: String) {
        let image = UIGraphicsImageRenderer(bounds: window.bounds).image { _ in
            window.drawHierarchy(in: window.bounds, afterScreenUpdates: true)
        }
        let attachment = XCTAttachment(image: image)
        attachment.name = "git-diff-\(name)"
        attachment.lifetime = .keepAlways
        add(attachment)
    }
}
