import SwiftUI
import XCTest
@testable import Collie

@MainActor
final class GlassPickerTests: XCTestCase {
    func testNativeSelectionAndLayouts() async throws {
        for style in [UIUserInterfaceStyle.light, .dark] {
            let state = PickerState()
            let host = UIHostingController(rootView: PickerFixture(state: state))
            host.overrideUserInterfaceStyle = style
            try await hosting(host) { window in
                window.layer.speed = 20
                let controls = try await waitFor("pickers") {
                    host.view.layoutIfNeeded()
                    let controls = descendants(host.view).compactMap { $0 as? UISegmentedControl }
                    return controls.count == 3 ? controls : nil
                }
                let compact = try XCTUnwrap(controls.first { $0.accessibilityLabel == "View" })
                let expanded = try XCTUnwrap(controls.first { $0.accessibilityLabel == "Session view" })
                let toolbar = try XCTUnwrap(controls.first { $0.accessibilityLabel == "Toolbar view" })
                XCTAssertGreaterThanOrEqual(compact.bounds.height, 44)
                XCTAssertLessThan(expanded.bounds.height, compact.bounds.height)
                XCTAssertEqual(expanded.bounds.width, 96)
                XCTAssertGreaterThanOrEqual(expanded.bounds.width / CGFloat(expanded.numberOfSegments), 44)
                XCTAssertEqual((0..<expanded.numberOfSegments).map { expanded.imageForSegment(at: $0)?.accessibilityLabel }, ["Chat", "Terminal"])
                XCTAssertEqual((0..<expanded.numberOfSegments).compactMap { expanded.titleForSegment(at: $0) }, [])
                XCTAssertGreaterThanOrEqual(compact.bounds.width, 150)
                XCTAssertEqual(compact.selectedSegmentIndex, 0)
                let attributes = try XCTUnwrap(compact.titleTextAttributes(for: .selected))
                try assertAppearance(controls, accent: state.accent, style: style)
                XCTAssertEqual((attributes[.font] as? UIFont)?.pointSize, 15)
                compact.selectedSegmentIndex = 1
                compact.sendActions(for: .valueChanged)
                XCTAssertTrue(state.selected)
                let image = try await waitFor("selection background") {
                    host.view.layoutIfNeeded()
                    let image = UIGraphicsImageRenderer(bounds: window.bounds).image { _ in
                        window.drawHierarchy(in: window.bounds, afterScreenUpdates: true)
                    }
                    let brightness = try? selectionBrightness(controls, image: image, window: window)
                    let selected = brightness?.allSatisfy { style == .dark ? $0 < 0.12 : $0 > 0.8 } == true
                    return expanded.selectedSegmentIndex == 1 && selected ? image : nil
                }
                let attachment = XCTAttachment(image: image)
                attachment.name = style == .dark ? "glass-picker-dark" : "glass-picker-light"
                attachment.lifetime = .keepAlways
                add(attachment)
                for accent in AppAccent.allCases {
                    state.accent = accent
                    toolbar.selectedSegmentIndex = 1 - toolbar.selectedSegmentIndex
                    toolbar.sendActions(for: .valueChanged)
                    _ = try await waitFor("compact selection") {
                        compact.selectedSegmentIndex == toolbar.selectedSegmentIndex ? true : nil
                    }
                    host.view.layoutIfNeeded()
                    try assertAppearance(controls, accent: accent, style: style)
                    for presented in [true, false] {
                        state.wrapped.toggle()
                        state.sheet = presented
                        _ = try await waitFor(presented ? "sheet" : "dismissal") {
                            guard let sheet = host.presentedViewController else { return presented ? nil : true }
                            return presented && !sheet.isBeingPresented && sheet.transitionCoordinator == nil ? true : nil
                        }
                        host.view.layoutIfNeeded()
                        try assertAppearance(controls, accent: accent, style: style)
                    }
                }
            }
        }
    }

    @Observable
    final class PickerState {
        var selected = false
        var wrapped = true
        var sheet = false
        var accent = AppAccent.gold
    }

    private struct PickerFixture: View {
        @Bindable var state: PickerState

        var body: some View {
            NavigationStack {
                VStack(spacing: 24) {
                    GlassPicker(title: "View", selection: $state.selected, options: [("Agents", false), ("Usage", true)])
                    GlassPicker(title: "Session view", selection: $state.selected, options: [("Chat", false), ("Terminal", true)],
                                expands: true, icons: [UIImage(systemName: "bubble.left.and.bubble.right"), UIImage(named: "Ghostty")])
                        .controlSize(.small)
                        .frame(width: 96)
                }
                .padding()
                .navigationTitle("Agents")
                .navigationBarTitleDisplayMode(.inline)
                .toolbar(removing: .title)
                .toolbar {
                    ToolbarItem(placement: .topBarLeading) {
                        GlassPicker(title: "Toolbar view", selection: $state.selected, options: [("Agents", false), ("Usage", true)])
                            .fixedSize()
                    }
                    .sharedBackgroundVisibility(.hidden)
                    ToolbarItem(placement: .primaryAction) {
                        Toggle(isOn: $state.wrapped) { Label("Wrap lines", systemImage: "text.word.spacing") }
                            .toggleStyle(.button)
                    }
                    ToolbarItem(placement: .primaryAction) {
                        Button("New task", systemImage: "plus") { state.sheet = true }
                    }
                }
            }
            .sheet(isPresented: $state.sheet) { Text("New task") }
            .tint(state.accent.color)
            .accentColor(state.accent.color)
            .environment(\.appAccent, state.accent.color)
        }
    }

    private func assertAppearance(_ controls: [UISegmentedControl], accent: AppAccent, style: UIUserInterfaceStyle) throws {
        let expected = UIColor(accent.color).resolvedColor(with: UITraitCollection(userInterfaceStyle: style))
        for control in controls {
            let background = try XCTUnwrap(control.selectedSegmentTintColor)
            XCTAssertEqual(background.resolvedColor(with: UITraitCollection(userInterfaceStyle: style)), style == .dark ? .black : .white)
            XCTAssertNil(control.titleTextAttributes(for: .normal)?[.foregroundColor])
            let actual = try XCTUnwrap(control.titleTextAttributes(for: .selected)?[.foregroundColor] as? UIColor)
            XCTAssertEqual(actual.cgColor.numberOfComponents, expected.cgColor.numberOfComponents)
            for (component, wanted) in zip(actual.cgColor.components ?? [], expected.cgColor.components ?? []) {
                XCTAssertEqual(component, wanted, accuracy: 0.002, "\(accent) in \(control.accessibilityLabel ?? "picker")")
            }
        }
    }

    private func selectionBrightness(_ controls: [UISegmentedControl], image: UIImage, window: UIWindow) throws -> [Double] {
        let cgImage = try XCTUnwrap(image.cgImage)
        return try controls.flatMap { control in
            let segmentWidth = control.bounds.width / CGFloat(control.numberOfSegments)
            let point = control.convert(
                CGPoint(x: segmentWidth * CGFloat(control.selectedSegmentIndex) + 10, y: control.bounds.midY),
                to: window
            )
            let sample = try XCTUnwrap(cgImage.cropping(to: CGRect(
                x: point.x * image.scale, y: point.y * image.scale, width: 1, height: 1
            )))
            var pixel = [UInt8](repeating: 0, count: 4)
            try pixel.withUnsafeMutableBytes { bytes in
                let context = try XCTUnwrap(CGContext(
                    data: bytes.baseAddress, width: 1, height: 1, bitsPerComponent: 8, bytesPerRow: 4,
                    space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
                ))
                context.draw(sample, in: CGRect(x: 0, y: 0, width: 1, height: 1))
            }
            return pixel.prefix(3).map { Double($0) / 255 }
        }
    }
}
