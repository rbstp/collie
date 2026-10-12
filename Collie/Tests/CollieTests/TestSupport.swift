import CollieCore
import Foundation
import Testing
import UIKit
import XCTest

@testable import Collie

@MainActor
func agentModel(_ core: FakeCore) -> AgentModel {
    AgentModel(core: core, route: AgentRoute(machineId: "m1", terminalId: "term_1"), prefsFile: nil)
}

func route(_ terminalId: String, on machineId: String = "m1") -> AgentRoute {
    AgentRoute(machineId: machineId, terminalId: terminalId)
}

func makeTemporaryDirectory() throws -> URL {
    let dir = FileManager.default.temporaryDirectory.appending(path: UUID().uuidString, directoryHint: .isDirectory)
    try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
    return dir
}

func prefsFile() throws -> URL {
    try makeTemporaryDirectory().appending(path: "prefs.json")
}

@MainActor
func hosting(_ host: UIViewController, body: @MainActor (UIWindow) async throws -> Void) async throws {
    let scene = try XCTUnwrap(UIApplication.shared.connectedScenes.first as? UIWindowScene)
    let window = UIWindow(windowScene: scene)
    window.frame = CGRect(x: 0, y: 0, width: 402, height: 874)
    window.rootViewController = host
    let previous = scene.keyWindow
    window.makeKeyAndVisible()
    defer {
        host.view.endEditing(true)
        window.isHidden = true
        window.rootViewController = nil
        previous?.makeKeyAndVisible()
    }
    try await body(window)
}

@MainActor
func waitFor<T>(_ description: String, value: () -> T?) async throws -> T {
    let deadline = ContinuousClock.now.advanced(by: .seconds(10))
    repeat {
        if let result = value() { return result }
        try await Task.sleep(for: .milliseconds(50))
    } while ContinuousClock.now < deadline
    return try XCTUnwrap(value(), "Timed out waiting for \(description)")
}

func poll(
    timeout: Duration = .seconds(10), isolation: isolated (any Actor)? = #isolation,
    sourceLocation: SourceLocation = #_sourceLocation, until condition: () -> Bool
) async {
    let deadline = ContinuousClock.now.advanced(by: timeout)
    while !condition() {
        guard ContinuousClock.now < deadline else {
            Issue.record("Timed out polling", sourceLocation: sourceLocation)
            return
        }
        try? await Task.sleep(for: .milliseconds(5))
    }
}

@MainActor
func descendants(_ view: UIView) -> [UIView] {
    [view] + view.subviews.flatMap(descendants)
}
