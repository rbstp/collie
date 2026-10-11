import SwiftUI
import XCTest
@testable import Collie

@MainActor
final class AttachmentMenuTests: XCTestCase {
    func testSmallerIconUsesAppAccent() async throws {
        let scene = try XCTUnwrap(UIApplication.shared.connectedScenes.first as? UIWindowScene)
        for style in [UIUserInterfaceStyle.light, .dark] {
            let host = UIHostingController(rootView: AttachmentMenu { _ in }
                .environment(\.appAccent, AppAccent.gold.color))
            host.overrideUserInterfaceStyle = style
            let window = UIWindow(windowScene: scene)
            window.frame = CGRect(x: 0, y: 0, width: 402, height: 874)
            window.rootViewController = host
            let previous = scene.keyWindow
            window.makeKeyAndVisible()
            defer {
                window.isHidden = true
                window.rootViewController = nil
                previous?.makeKeyAndVisible()
            }
            try await Task.sleep(for: .milliseconds(250))
            host.view.layoutIfNeeded()
            let button = try XCTUnwrap(descendants(host.view).first { $0 is AttachmentMenuButton } as? AttachmentMenuButton)
            XCTAssertGreaterThanOrEqual(button.bounds.width, 44)
            XCTAssertGreaterThanOrEqual(button.bounds.height, 44)
            let imageView = try XCTUnwrap(button.imageView)
            XCTAssertLessThanOrEqual(imageView.bounds.height, 26)
            XCTAssertEqual(imageView.image?.renderingMode, .alwaysOriginal)
            let image = UIGraphicsImageRenderer(bounds: window.bounds).image { _ in
                window.drawHierarchy(in: window.bounds, afterScreenUpdates: true)
            }
            let attachment = XCTAttachment(image: image)
            attachment.name = style == .dark ? "attachment-icon-dark" : "attachment-icon-light"
            attachment.lifetime = .keepAlways
            add(attachment)
            let actual = try XCTUnwrap(button.configuration?.baseForegroundColor).resolvedColor(with: button.traitCollection)
            let expected = UIColor(AppAccent.gold.color).resolvedColor(with: button.traitCollection)
            XCTAssertEqual(actual.cgColor.numberOfComponents, expected.cgColor.numberOfComponents)
            for (component, wanted) in zip(actual.cgColor.components ?? [], expected.cgColor.components ?? []) {
                XCTAssertEqual(component, wanted, accuracy: 0.002)
            }
        }
    }

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

    private func descendants(_ view: UIView) -> [UIView] {
        [view] + view.subviews.flatMap(descendants)
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
