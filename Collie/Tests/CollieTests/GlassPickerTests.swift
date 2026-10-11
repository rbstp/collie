import SwiftUI
import XCTest
@testable import Collie

@MainActor
final class GlassPickerTests: XCTestCase {
    func testNativeSelectionAndLayouts() async throws {
        let scene = try XCTUnwrap(UIApplication.shared.connectedScenes.first as? UIWindowScene)
        for style in [UIUserInterfaceStyle.light, .dark] {
            let state = PickerState()
            let host = UIHostingController(rootView: PickerFixture(state: state))
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
            let controls = descendants(host.view).compactMap { $0 as? UISegmentedControl }
            XCTAssertEqual(controls.count, 2)
            let compact = try XCTUnwrap(controls.first)
            let expanded = try XCTUnwrap(controls.last)
            XCTAssertLessThan(compact.bounds.width, expanded.bounds.width)
            XCTAssertGreaterThanOrEqual(compact.bounds.height, 44)
            XCTAssertGreaterThanOrEqual(compact.bounds.width, 150)
            XCTAssertEqual(compact.selectedSegmentIndex, 0)
            let attributes = try XCTUnwrap(compact.titleTextAttributes(for: .selected))
            let color = try XCTUnwrap(attributes[.foregroundColor] as? UIColor)
            let expected = UIColor(AppAccent.gold.color).resolvedColor(with: UITraitCollection(userInterfaceStyle: style))
            for (actual, wanted) in zip(color.cgColor.components ?? [], expected.cgColor.components ?? []) {
                XCTAssertEqual(actual, wanted, accuracy: 0.002)
            }
            XCTAssertEqual((attributes[.font] as? UIFont)?.pointSize, 15)
            compact.selectedSegmentIndex = 1
            compact.sendActions(for: .valueChanged)
            XCTAssertTrue(state.selected)
            try await Task.sleep(for: .milliseconds(250))
            host.view.layoutIfNeeded()
            XCTAssertEqual(expanded.selectedSegmentIndex, 1)
            let image = UIGraphicsImageRenderer(bounds: window.bounds).image { _ in
                window.drawHierarchy(in: window.bounds, afterScreenUpdates: true)
            }
            let attachment = XCTAttachment(image: image)
            attachment.name = style == .dark ? "glass-picker-dark" : "glass-picker-light"
            attachment.lifetime = .keepAlways
            add(attachment)
            state.accent = .red
            state.selected = false
            try await Task.sleep(for: .milliseconds(250))
            host.view.layoutIfNeeded()
            XCTAssertEqual(compact.selectedSegmentIndex, 0)
            let updated = try XCTUnwrap(compact.titleTextAttributes(for: .selected)?[.foregroundColor] as? UIColor)
            XCTAssertNotEqual(updated, color)
        }
    }

    @Observable
    final class PickerState {
        var selected = false
        var accent = AppAccent.gold
    }

    private struct PickerFixture: View {
        @Bindable var state: PickerState

        var body: some View {
            VStack(spacing: 24) {
                GlassPicker(title: "View", selection: $state.selected, options: [("Agents", false), ("Usage", true)])
                GlassPicker(title: "Session view", selection: $state.selected, options: [("Chat", false), ("Terminal", true)], expands: true)
            }
            .padding()
            .tint(state.accent.color)
            .accentColor(state.accent.color)
        }
    }

    private func descendants(_ view: UIView) -> [UIView] {
        [view] + view.subviews.flatMap(descendants)
    }
}
