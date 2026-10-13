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
            let file = GitFile(path: "Collie/UsageSection.swift", oldPath: nil, section: "unstaged", status: "M", additions: 1, deletions: 1, hash: nil)
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

    func testScopePickerLoadsTheBranchChanges() async throws {
        let core = FakeCore()
        core.state.withLock {
            $0.gitBase = "origin/main"
            $0.gitFiles = [
                GitFile(path: "a.swift", oldPath: nil, section: "unstaged", status: "M", additions: 1, deletions: 1, hash: "a"),
                GitFile(path: "b.swift", oldPath: nil, section: "committed", status: "A", additions: 2, deletions: 0, hash: "b")
            ]
        }
        let model = AgentModel(core: core, route: AgentRoute(machineId: "m", terminalId: "t"), prefsFile: nil)
        let host = UIHostingController(rootView: GitChangesView(agent: model, sent: {}, review: model.gitReview))
        try await hosting(host) { window in
            let picker = try await waitFor("scope picker") {
                host.view.layoutIfNeeded()
                let picker = descendants(host.view).compactMap { $0 as? UISegmentedControl }.first
                return picker?.isEnabled == true && core.snapshot.gitCalls == ["m|t"] ? picker : nil
            }
            XCTAssertEqual((0..<picker.numberOfSegments).map { picker.titleForSegment(at: $0) }, ["Uncommitted", "Branch"])
            XCTAssertEqual(picker.selectedSegmentIndex, 0)
            snapshot(window, name: "uncommitted")
            picker.selectedSegmentIndex = 1
            picker.sendActions(for: .valueChanged)
            _ = try await waitFor("branch changes") {
                host.view.layoutIfNeeded()
                return core.snapshot.gitCalls == ["m|t", "m|t|branch"] && picker.isEnabled ? true : nil
            }
            snapshot(window, name: "branch")
        }
    }

    func testViewedMarksPersistSortLastCollapseAndClearWhenTheDiffChanges() async throws {
        let file = FileManager.default.temporaryDirectory.appending(path: UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: file) }
        let route = AgentRoute(machineId: "m", terminalId: "t")
        let first = GitFile(path: "a.swift", oldPath: nil, section: "unstaged", status: "M", additions: 1, deletions: 1, hash: "1")
        let second = GitFile(path: "b.swift", oldPath: nil, section: "unstaged", status: "M", additions: 1, deletions: 1, hash: "2")
        let review = GitReview(viewedFile: file)
        review.setViewed(true, route: route, root: "/worktree", file: first)
        XCTAssertEqual(review.ordered([first, second], route: route, root: "/worktree").map(\.file.path), ["b.swift", "a.swift"])
        XCTAssertEqual(review.ordered([first, second], route: route, root: "/worktree").map(\.viewed), [false, true])

        let reloaded = GitReview(viewedFile: file)
        XCTAssertTrue(reloaded.isViewed(route: route, root: "/worktree", file: first))
        XCTAssertFalse(reloaded.isViewed(route: route, root: "/other", file: first))
        XCTAssertFalse(reloaded.isViewed(route: AgentRoute(machineId: "m", terminalId: "u"), root: "/worktree", file: first))
        var changed = first
        changed.hash = "3"
        XCTAssertFalse(reloaded.isViewed(route: route, root: "/worktree", file: changed))
        changed.hash = nil
        XCTAssertFalse(reloaded.isViewed(route: route, root: "/worktree", file: changed))

        let core = FakeCore()
        let model = AgentModel(core: core, route: route, prefsFile: nil, viewedFile: file)
        for (gitFile, collapsed) in [(second, false), (first, true)] {
            let host = UIHostingController(rootView: NavigationStack {
                GitDiffView(agent: model, root: "/worktree", file: gitFile, sent: {}, review: model.gitReview)
            })
            try await hosting(host) { _ in
                _ = try await waitFor("\(gitFile.path) diff") {
                    core.snapshot.gitCalls.last == "m|t|/worktree|\(gitFile.path)|unstaged" ? true : nil
                }
                let shown = { descendants(host.view).contains { ($0 as? UITextView)?.text.contains("new") == true } }
                if collapsed {
                    try await Task.sleep(for: .milliseconds(500))
                    host.view.layoutIfNeeded()
                    XCTAssertFalse(shown(), gitFile.path)
                } else {
                    _ = try await waitFor("\(gitFile.path) expanded") {
                        host.view.layoutIfNeeded()
                        return shown() ? true : nil
                    }
                }
            }
        }

        reloaded.setViewed(false, route: route, root: "/worktree", file: first)
        XCTAssertFalse(GitReview(viewedFile: file).isViewed(route: route, root: "/worktree", file: first))
        reloaded.setViewed(true, route: route, root: "/worktree", file: first)
        GitViewed.forget(machineId: "m", file: file)
        XCTAssertFalse(GitReview(viewedFile: file).isViewed(route: route, root: "/worktree", file: first))
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
