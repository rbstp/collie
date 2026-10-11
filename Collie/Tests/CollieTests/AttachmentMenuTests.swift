import SwiftUI
import XCTest
@testable import Collie

@MainActor
final class AttachmentMenuTests: XCTestCase {
    func testEveryChoiceWaitsForMenuDismissal() {
        for choice in AttachmentChoice.allCases {
            let button = AttachmentMenuButton()
            button.configure(savedPromptsEnabled: true, filesEnabled: true, accent: .systemBlue, locale: .current)
            var selected: [AttachmentChoice] = []
            button.select = { selected.append($0) }
            let animator = MenuAnimator()
            button.queueSelection(choice)
            XCTAssertTrue(selected.isEmpty)
            button.finishSelection(animator: animator)
            XCTAssertTrue(selected.isEmpty)
            XCTAssertNil(button.pendingChoice)
            animator.complete()
            XCTAssertEqual(selected, [choice])
            button.finishSelection(animator: animator)
            animator.complete()
            XCTAssertEqual(selected, [choice])
        }
    }

    func testCancellationAndDisabledChoicesDoNotPresent() {
        let button = AttachmentMenuButton()
        button.configure(savedPromptsEnabled: true, filesEnabled: false, accent: .systemBlue, locale: .current)
        var selected: [AttachmentChoice] = []
        button.select = { selected.append($0) }
        button.finishSelection(animator: nil)
        for choice in [AttachmentChoice.files, .photos] {
            button.queueSelection(choice)
            button.finishSelection(animator: nil)
        }
        XCTAssertTrue(selected.isEmpty)
        button.queueSelection(.savedPrompts)
        button.finishSelection(animator: nil)
        XCTAssertEqual(selected, [.savedPrompts])
        button.configure(savedPromptsEnabled: false, filesEnabled: true, accent: .systemBlue, locale: .current)
        button.queueSelection(.savedPrompts)
        XCTAssertNil(button.pendingChoice)
        button.queueSelection(.files)
        let animator = MenuAnimator()
        button.finishSelection(animator: animator)
        button.isEnabled = false
        animator.complete()
        XCTAssertEqual(selected, [.savedPrompts])
    }

    func testPresentationUsesLatestCallbackAfterUpdates() {
        let button = AttachmentMenuButton()
        var selected: [String] = []
        button.select = { _ in selected.append("old") }
        button.queueSelection(.files)
        let animator = MenuAnimator()
        button.finishSelection(animator: animator)
        button.select = { _ in selected.append("current") }
        animator.complete()
        XCTAssertEqual(selected, ["current"])
    }

    private final class MenuAnimator: NSObject, UIContextMenuInteractionAnimating {
        var previewViewController: UIViewController? { nil }
        private var completions: [() -> Void] = []

        func addAnimations(_ animations: @escaping () -> Void) { animations() }
        func addCompletion(_ completion: @escaping () -> Void) { completions.append(completion) }

        func complete() {
            let pending = completions
            completions = []
            pending.forEach { $0() }
        }
    }
}
