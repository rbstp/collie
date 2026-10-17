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

    func testResumeListsTheFoldersSessionsAndStartsWithoutAPrompt() async throws {
        let core = FakeCore()
        let machine = Machine(id: "m1", label: "Mac", host: "mac.ts.net", port: 8457, nodeId: "n1", kind: .mac, key: "")
        let model = NewTaskModel(core: core, machines: [machine], prefsFile: nil)
        await model.loadOptions()
        model.location = .resume
        model.cwd = "/Users/me/app"
        let host = UIHostingController(rootView: NewTaskSheet(model: model, machines: [machine]) { _ in })
        try await hosting(host) { _ in
            let sessions = try await waitFor("sessions listed by the sheet") { model.sessions }
            XCTAssertEqual(sessions.map(\.firstPrompt), ["add approval tests", "fix the flaky test"])
            XCTAssertEqual(core.snapshot.sessionCalls, ["/Users/me/app"])
            XCTAssertFalse(model.canStart, "no session picked")
            model.session = sessions[1]
            XCTAssertTrue(model.canStart, "the prompt is optional")
            model.cwd = "/Users/me/other"
            XCTAssertNil(model.session)
            XCTAssertFalse(model.canStart)
            let other = try await waitFor("the other folder's sessions") {
                core.snapshot.sessionCalls.last == "/Users/me/other" ? model.sessions : nil
            }
            model.session = other[1]
            XCTAssertTrue(model.canStart)
            core.state.withLock { $0.started = TaskStarted(workspaceId: "w1", terminalId: "term_new") }
            let route = await model.start()
            XCTAssertEqual(route, AgentRoute(machineId: "m1", terminalId: "term_new"))
            XCTAssertEqual(core.snapshot.taskNews, ["/Users/me/other resume=claude:3f1b2c4d-5e6f-4a7b-8c9d-0e1f2a3b4c5d"])
            XCTAssertEqual(core.snapshot.taskPrompts, [""])
        }
    }

    func testUltracodeShowsOnlyForClaudeCodeAndIsRememberedPerMachine() async throws {
        let core = FakeCore()
        let mac = Machine(id: "m1", label: "Mac", host: "mac.ts.net", port: 8457, nodeId: "n1", kind: .mac, key: "")
        let linux = Machine(id: "m2", label: "omarchy", host: "omarchy.ts.net", port: 8457, nodeId: "n2", kind: .linux, key: "")
        let prefs = FileManager.default.temporaryDirectory.appending(path: "prefs-\(UUID().uuidString).json")
        defer { try? FileManager.default.removeItem(at: prefs) }
        core.state.withLock { $0.options.agents.append("copilot") }
        let model = NewTaskModel(core: core, machines: [mac, linux], prefsFile: prefs)
        await model.loadOptions()
        XCTAssertEqual(model.agent, "codex")
        XCTAssertFalse(model.ultracode)
        model.prompt = "go"
        model.cwd = "/Users/me/app"
        let host = UIHostingController(rootView: NewTaskSheet(model: model, machines: [mac, linux]) { _ in })
        try await hosting(host) { _ in
            let switches = {
                host.view.layoutIfNeeded()
                return descendants(host.view).filter { view in
                    view is UISwitch && view.window != nil
                        && sequence(first: view, next: \.superview).allSatisfy { !$0.isHidden && $0.alpha > 0 }
                }.count
            }
            _ = try await waitFor("only the new folder switch") { switches() == 1 ? true : nil }
            model.agent = "claude"
            _ = try await waitFor("the ultracode switch") { switches() == 2 ? true : nil }
            model.ultracode = true
            model.agent = "copilot"
            _ = try await waitFor("no ultracode switch for Copilot CLI") { switches() == 1 ? true : nil }
            model.agent = "claude"
            core.state.withLock { $0.started = TaskStarted(workspaceId: "w1", terminalId: "term_new") }
            let route = await model.start()
            XCTAssertEqual(route, AgentRoute(machineId: "m1", terminalId: "term_new"))
            XCTAssertEqual(core.snapshot.taskNews, ["/Users/me/app - ultracode"])
        }
        XCTAssertEqual(DevicePrefs.load(from: prefs).taskUltracode, ["m1": true])

        let next = NewTaskModel(core: core, machines: [mac, linux], preferredMachineId: "m2", prefsFile: prefs)
        await next.loadOptions()
        XCTAssertFalse(next.ultracode)
        next.machineId = "m1"
        await next.loadOptions()
        XCTAssertTrue(next.ultracode)
        XCTAssertEqual(next.agent, "codex")
        next.prompt = "go"
        next.cwd = "/Users/me/app"
        _ = await next.start()
        XCTAssertEqual(core.snapshot.taskNews.last, "/Users/me/app -", "only Claude Code gets ultracode")
    }

    func testPullRequestTakesANumberAndStartsOnItsBranch() async throws {
        let core = FakeCore()
        let machine = Machine(id: "m1", label: "Mac", host: "mac.ts.net", port: 8457, nodeId: "n1", kind: .mac, key: "")
        let model = NewTaskModel(core: core, machines: [machine], prefsFile: nil)
        await model.loadOptions()
        model.prompt = "fix it"
        model.location = .worktree
        model.worktreeAction = .pullRequest
        model.source = "/Users/me/app"
        let host = UIHostingController(rootView: NewTaskSheet(model: model, machines: [machine]) { _ in })
        try await hosting(host) { _ in
            let field = try await waitFor("pull request number field") {
                host.view.layoutIfNeeded()
                return descendants(host.view).compactMap { $0 as? UITextField }.first { $0.placeholder == "Pull request number" }
            }
            XCTAssertEqual(field.keyboardType, .numberPad)
            for bad in ["", "0", "-3", "+3", "3.5", "12a", "٣", "4294967296"] {
                model.pullRequestNumber = bad
                XCTAssertNil(model.pullRequest, bad)
                XCTAssertFalse(model.canStart, bad)
            }
            model.pullRequestNumber = " 282 "
            XCTAssertEqual(model.pullRequest, 282)
            XCTAssertTrue(model.canStart)
            core.state.withLock { $0.started = TaskStarted(workspaceId: "w1", terminalId: "term_new") }
            let route = await model.start()
            XCTAssertEqual(route, AgentRoute(machineId: "m1", terminalId: "term_new"))
            XCTAssertEqual(core.snapshot.worktreeCalls, ["list /Users/me/app", "pull_request /Users/me/app 282 codex"])
            XCTAssertEqual(core.snapshot.taskPrompts, ["fix it"])
        }
    }

    private func controllers(_ controller: UIViewController) -> [UIViewController] {
        [controller] + controller.children.flatMap(controllers)
    }
}
