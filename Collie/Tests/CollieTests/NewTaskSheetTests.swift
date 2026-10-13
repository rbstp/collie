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
            let root = try XCTUnwrap(navigation.topViewController)
            let folders = UIHostingController(rootView: Text("Folders"))
            navigation.pushViewController(folders, animated: false)
            _ = try await waitFor("folders") { folders.view.window != nil && root.view.window == nil ? true : nil }
            XCTAssertFalse(model.cancelled)
            navigation.popViewController(animated: false)
            _ = try await waitFor("sheet root") { root.view.window != nil && folders.view.window == nil ? true : nil }
            XCTAssertFalse(model.cancelled)
            XCTAssertTrue(model.canStart)
            XCTAssertEqual(model.prompt, "keep this prompt")
            XCTAssertTrue(model.canStart)
            core.state.withLock { $0.started = TaskStarted(workspaceId: "w1", terminalId: "term_new") }
            let route = await model.start()
            XCTAssertEqual(route, AgentRoute(machineId: "m1", terminalId: "term_new"))
        }
    }

    func testCloneTakesARepositoryAndShowsProgressWhileCloning() async throws {
        let core = FakeCore()
        let machine = Machine(id: "m1", label: "Mac", host: "mac.ts.net", port: 8457, nodeId: "n1", kind: .mac, key: "")
        let model = NewTaskModel(core: core, machines: [machine], prefsFile: nil)
        await model.loadOptions()
        model.prompt = "set it up"
        model.location = .clone
        let host = UIHostingController(rootView: NewTaskSheet(model: model, machines: [machine]) { _ in })
        try await hosting(host) { _ in
            let field = try await waitFor("repository field") {
                host.view.layoutIfNeeded()
                return descendants(host.view).compactMap { $0 as? UITextField }.first { $0.placeholder == "owner/repo or URL" }
            }
            XCTAssertEqual(field.keyboardType, .URL)
            for bad in ["", "app", "file:///tmp/app", "-u/app", "https://user@github.com/owner/app"] {
                model.repository = bad
                XCTAssertNil(model.cloneFolder, bad)
                XCTAssertFalse(model.canStart, bad)
            }
            model.repository = " owner/app.git "
            XCTAssertEqual(model.cloneFolder, "app")
            _ = try await waitFor("options reloaded by the sheet") { model.canStart ? true : nil }
            core.state.withLock { $0.started = TaskStarted(workspaceId: "w1", terminalId: "term_new") }
            core.set(hold: true)
            let start = Task { await model.start() }
            await core.waitHeld(1)
            XCTAssertEqual(model.cloning, "owner/app.git")
            core.release()
            let route = await start.value
            XCTAssertNil(model.cloning)
            XCTAssertEqual(route, AgentRoute(machineId: "m1", terminalId: "term_new"))
            XCTAssertEqual(core.snapshot.taskNews, ["/Users/me clone=owner/app.git"])
        }
    }

    private func controllers(_ controller: UIViewController) -> [UIViewController] {
        [controller] + controller.children.flatMap(controllers)
    }
}
