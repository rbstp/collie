import SwiftUI
import XCTest
@testable import Collie

@MainActor
final class AttachmentMenuTests: XCTestCase {
    func testCircleMatchesNeighborsAndIconUsesAppAccent() async throws {
        for style in [UIUserInterfaceStyle.light, .dark] {
            let neighbor = FrameBox()
            let host = UIHostingController(rootView: HStack(alignment: .bottom, spacing: 8) {
                AttachmentMenu { _ in }
                Button {} label: {
                    Image(systemName: "mic").font(.system(size: 20)).frame(width: 28, height: 28)
                }
                .buttonStyle(.glass)
                .buttonBorderShape(.circle)
                .controlSize(.small)
                .onGeometryChange(for: CGRect.self) { $0.frame(in: .global) } action: { neighbor.frame = $0 }
            }
            .environment(\.appAccent, AppAccent.gold.color))
            host.overrideUserInterfaceStyle = style
            try await hosting(host) { window in
                try await Task.sleep(for: .milliseconds(250))
                host.view.layoutIfNeeded()
                let button = try XCTUnwrap(descendants(host.view).first { $0 is AttachmentMenuButton } as? AttachmentMenuButton)
                XCTAssertGreaterThanOrEqual(button.bounds.width, 44)
                XCTAssertGreaterThanOrEqual(button.bounds.height, 44)
                let insets = try XCTUnwrap(button.configuration?.background.backgroundInsets)
                let circle = button.convert(button.bounds.inset(by: UIEdgeInsets(
                    top: insets.top, left: insets.leading, bottom: insets.bottom, right: insets.trailing
                )), to: window)
                XCTAssertFalse(neighbor.frame.isEmpty)
                XCTAssertEqual(circle.width, neighbor.frame.width, accuracy: 0.5)
                XCTAssertEqual(circle.height, neighbor.frame.height, accuracy: 0.5)
                XCTAssertEqual(circle.midY, neighbor.frame.midY, accuracy: 0.5)
                for point in [CGPoint(x: circle.minX - 2, y: circle.midY), CGPoint(x: circle.maxX + 2, y: circle.midY),
                              CGPoint(x: circle.midX, y: circle.minY - 2), CGPoint(x: circle.midX, y: circle.maxY + 2)] {
                    XCTAssertTrue(window.hitTest(point, with: nil)?.isDescendant(of: button) == true, "\(point)")
                }
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

    private final class FrameBox {
        var frame = CGRect.null
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
