import SwiftUI
import XCTest
@testable import Collie

@MainActor
final class GlassPickerTests: XCTestCase {
    func testNativeSelectionAndLayouts() async throws {
        let scene = try XCTUnwrap(UIApplication.shared.connectedScenes.first as? UIWindowScene)
        for style in [UIUserInterfaceStyle.light, .dark] {
            var selected = false
            let selection = Binding(get: { selected }, set: { selected = $0 })
            let host = UIHostingController(rootView: VStack(spacing: 24) {
                GlassPicker(title: "View", selection: selection, options: [("Agents", false), ("Usage", true)])
                GlassPicker(title: "Session view", selection: selection, options: [("Chat", false), ("Terminal", true)], expands: true)
            }
            .padding()
            .tint(AppAccent.gold.color)
            .accentColor(AppAccent.gold.color))
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
            XCTAssertEqual(compact.selectedSegmentIndex, 0)
            compact.selectedSegmentIndex = 1
            compact.sendActions(for: .valueChanged)
            XCTAssertTrue(selected)
            let image = UIGraphicsImageRenderer(bounds: window.bounds).image { _ in
                window.drawHierarchy(in: window.bounds, afterScreenUpdates: true)
            }
            let attachment = XCTAttachment(image: image)
            attachment.name = style == .dark ? "glass-picker-dark" : "glass-picker-light"
            attachment.lifetime = .keepAlways
            add(attachment)
        }
    }

    private func descendants(_ view: UIView) -> [UIView] {
        [view] + view.subviews.flatMap(descendants)
    }
}
