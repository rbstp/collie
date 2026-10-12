import CollieCore
import SwiftUI
import XCTest
@testable import Collie

@MainActor
final class NewTaskSheetTests: XCTestCase {
    func testBrowsingDoesNotCancelTheTask() async throws {
        let core = FakeCore()
        let machine = Machine(id: "m1", label: "Mac", host: "mac.ts.net", port: 8457, nodeId: "n1", kind: .mac, key: "")
        let model = NewTaskModel(core: core, machines: [machine], prefsFile: nil)
        await model.loadOptions()
        model.prompt = "keep this prompt"
        model.cwd = "/Users/me/app"
        let host = UIHostingController(rootView: NewTaskSheet(model: model, machines: [machine]) { _ in })
        try await hosting(host) { _ in
            _ = try await waitFor("prompt field") {
                host.view.layoutIfNeeded()
                return descendants(host.view).compactMap { $0 as? UITextView }.first { $0.text == model.prompt }
            }
            let navigation = try XCTUnwrap(controllers(host).compactMap { $0 as? UINavigationController }.first)
            navigation.pushViewController(UIHostingController(rootView: Text("Folders")), animated: false)
            try await Task.sleep(for: .milliseconds(250))
            XCTAssertFalse(model.cancelled)
            navigation.popViewController(animated: false)
            try await Task.sleep(for: .milliseconds(250))
            XCTAssertFalse(model.cancelled)
            XCTAssertTrue(model.canStart)
            XCTAssertEqual(model.prompt, "keep this prompt")
            XCTAssertTrue(model.canStart)
            core.state.withLock { $0.started = TaskStarted(workspaceId: "w1", terminalId: "term_new") }
            let route = await model.start()
            XCTAssertEqual(route, AgentRoute(machineId: "m1", terminalId: "term_new"))
        }
    }

    private func controllers(_ controller: UIViewController) -> [UIViewController] {
        [controller] + controller.children.flatMap(controllers)
    }
}
