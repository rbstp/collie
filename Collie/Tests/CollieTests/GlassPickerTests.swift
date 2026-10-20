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
                XCTAssertEqual((0..<expanded.numberOfSegments).map { expanded.widthForSegment(at: $0) }, [44, 44])
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
                _ = try await waitFor("selection") {
                    host.view.layoutIfNeeded()
                    return expanded.selectedSegmentIndex == 1 ? true : nil
                }
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
                                icons: [UIImage(systemName: "bubble.left.and.bubble.right"), UIImage(named: "Ghostty")])
                        .controlSize(.small)
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
            XCTAssertNil(control.selectedSegmentTintColor)
            XCTAssertNil(control.titleTextAttributes(for: .normal)?[.foregroundColor])
            let actual = try XCTUnwrap(control.titleTextAttributes(for: .selected)?[.foregroundColor] as? UIColor)
            XCTAssertEqual(actual.cgColor.numberOfComponents, expected.cgColor.numberOfComponents)
            for (component, wanted) in zip(actual.cgColor.components ?? [], expected.cgColor.components ?? []) {
                XCTAssertEqual(component, wanted, accuracy: 0.002, "\(accent) in \(control.accessibilityLabel ?? "picker")")
            }
        }
    }
}
