import CollieCore
import Foundation
import Synchronization
import Testing

@testable import Collie

final class FakeCore: AgentCore {
    struct State {
        var prompts: [String] = []
        var keys: [[AgentKey]] = []
        var closes: [String] = []
        var hold = false
        var held: [CheckedContinuation<Void, Never>] = []
        var error: CoreError?
        var options = TaskOptions(agents: ["claude", "codex"], defaultAgent: "codex", recentCwds: ["/Users/me/app"])
        var started: TaskStarted?
        var uploads: [String] = []
        var cancelledUploads: [String] = []
        var maxAttachmentBytes: UInt64 = 20 * 1024 * 1024
        var uploadPath = "/Users/me/Library/Caches/dev.rbstp.collied/attachments/0123456789abcdef/notes.txt"
    }

    let state = Mutex(State())

    var snapshot: State { state.withLock { $0 } }

    func set(hold: Bool = false, error: CoreError? = nil) {
        state.withLock {
            $0.hold = hold
            $0.error = error
        }
    }

    func waitHeld(_ count: Int) async {
        while state.withLock({ $0.held.count }) < count {
            try? await Task.sleep(for: .milliseconds(5))
        }
    }

    func release() {
        let held = state.withLock { s in
            s.hold = false
            let held = s.held
            s.held = []
            return held
        }
        held.forEach { $0.resume() }
    }

    private func call(_ record: @Sendable (inout State) -> Void) async throws {
        let hold = state.withLock { s in
            record(&s)
            return s.hold
        }
        if hold {
            await withCheckedContinuation { continuation in
                state.withLock { $0.held.append(continuation) }
            }
        }
        if let error = state.withLock({ $0.error }) { throw error }
    }

    func agentView(machineId: String, terminalId: String, afterRevision: UInt64) -> AgentView? { nil }
    func watchAgent(machineId: String, terminalId: String?) async throws {}
    func agentRead(machineId: String, terminalId: String, source: TerminalSource) async throws -> TerminalSnapshot {
        TerminalSnapshot(terminalId: terminalId, source: source, ansi: "", truncated: false)
    }
    func prompt(machineId: String, terminalId: String, text: String) async throws {
        try await call { $0.prompts.append(text) }
    }
    func sendKeys(machineId: String, terminalId: String, keys: [AgentKey]) async throws {
        try await call { $0.keys.append(keys) }
    }
    func focus(machineId: String, terminalId: String) async throws {}
    func closeWorkspace(machineId: String, workspaceId: String, confirm: Bool) async throws {
        try await call { $0.closes.append("workspace \(workspaceId) confirm=\(confirm)") }
    }
    func closePane(machineId: String, terminalId: String, confirm: Bool) async throws {
        try await call { $0.closes.append("pane \(terminalId) confirm=\(confirm)") }
    }
    func taskOptions(machineId: String) async throws -> TaskOptions { state.withLock { $0.options } }
    func taskNew(machineId: String, cwd: String, agent: String, prompt: String, label: String?) async throws -> TaskStarted {
        try await call { _ in }
        guard let started = state.withLock({ $0.started }) else { throw CoreError.NotImplemented }
        return started
    }
    func uploadAttachment(machineId: String, name: String, data: Data, progress: any UploadProgress) async throws -> String {
        progress.onProgress(sent: UInt64(data.count / 2), total: UInt64(data.count))
        try await call { $0.uploads.append("\(name) \(data.count)") }
        progress.onProgress(sent: UInt64(data.count), total: UInt64(data.count))
        return state.withLock { $0.uploadPath }
    }
    func maxAttachmentBytes() -> UInt64 { state.withLock { $0.maxAttachmentBytes } }
    func cancelUploads(machineId: String) { state.withLock { $0.cancelledUploads.append(machineId) } }
    func flock(machineId: String) async throws -> MachineFlock {
        guard let started = state.withLock({ $0.started }) else { throw CoreError.MachineNotFound }
        let machine = Machine(id: machineId, label: "Mac", host: "mac.ts.net", port: 8457, nodeId: "n1")
        let agent = AgentSummary(
            terminalId: started.terminalId, workspaceId: started.workspaceId, kind: "claude", name: nil, title: nil,
            status: .working, statusSinceMs: 0, cwd: nil, lastLine: nil
        )
        return MachineFlock(machine: machine, link: .connected, lastError: nil, details: nil, workspaces: [], agents: [agent], approvalsCount: 0)
    }
}

@MainActor
private func agentModel(_ core: FakeCore) -> AgentModel {
    AgentModel(core: core, route: AgentRoute(machineId: "m1", terminalId: "term_1"), prefsFile: nil)
}

@MainActor
@Test func promptIsDisabledWhileSendingAndClearedOnSuccess() async {
    let core = FakeCore()
    let model = agentModel(core)
    #expect(!model.canSendPrompt)
    model.draft = "  \n "
    #expect(!model.canSendPrompt)
    model.draft = " fix the build\n"
    #expect(model.canSendPrompt)

    core.set(hold: true)
    let send = Task { await model.sendPrompt() }
    await core.waitHeld(1)
    #expect(model.sendingPrompt)
    #expect(!model.canSendPrompt)
    core.release()
    await send.value
    #expect(!model.sendingPrompt)
    #expect(model.draft.isEmpty)
    #expect(model.promptError == nil)
    #expect(core.snapshot.prompts == ["fix the build"])
}

@MainActor
@Test func textTypedWhileSendingIsKept() async {
    let core = FakeCore()
    let model = agentModel(core)
    model.draft = "one"
    core.set(hold: true)
    let send = Task { await model.sendPrompt() }
    await core.waitHeld(1)
    model.draft = "one and two"
    core.release()
    await send.value
    #expect(core.snapshot.prompts == ["one"])
    #expect(model.draft == "and two")

    model.draft = "three"
    core.set(hold: true)
    let replaced = Task { await model.sendPrompt() }
    await core.waitHeld(1)
    model.draft = "four"
    core.release()
    await replaced.value
    #expect(model.draft == "four")
}

@MainActor
private func attach(_ model: AgentModel, _ core: FakeCore, _ names: String...) async {
    for name in names {
        core.state.withLock { $0.uploadPath = "/Users/me/Library/Caches/dev.rbstp.collied/attachments/\(name)/\(name)" }
        await model.attach(name: name) { _ in Data([1]) }?.value
    }
}

@MainActor
@Test func attachmentPathsLeadThePromptAndAreClearedOnSuccess() async {
    let core = FakeCore()
    let model = agentModel(core)
    await attach(model, core, "a.png", "b.txt")
    model.draft = "  compare these\n"
    #expect(model.canSendPrompt)
    await model.sendPrompt()
    let a = "/Users/me/Library/Caches/dev.rbstp.collied/attachments/a.png/a.png"
    let b = "/Users/me/Library/Caches/dev.rbstp.collied/attachments/b.txt/b.txt"
    #expect(core.snapshot.prompts == ["\(a) \(b) compare these"])
    #expect(model.attachments.isEmpty)
    #expect(model.draft.isEmpty)
    #expect(!model.canSendPrompt)
}

@MainActor
@Test func attachmentsAloneCanBeSent() async {
    let core = FakeCore()
    let model = agentModel(core)
    await attach(model, core, "a.png")
    model.draft = " \n"
    #expect(model.canSendPrompt)
    await model.sendPrompt()
    #expect(core.snapshot.prompts == ["/Users/me/Library/Caches/dev.rbstp.collied/attachments/a.png/a.png"])
    #expect(model.attachments.isEmpty)
}

@MainActor
@Test func failedSendKeepsTheAttachmentsAndDraft() async {
    let core = FakeCore()
    let model = agentModel(core)
    await attach(model, core, "a.png")
    model.draft = "look"
    core.set(error: .AgentBlocked)
    await model.sendPrompt()
    #expect(model.attachments.map(\.name) == ["a.png"])
    #expect(model.draft == "look")
    #expect(model.attachmentSlots == 9)
}

@MainActor
@Test func filesAttachedWhileSendingAreKept() async {
    let core = FakeCore()
    let model = agentModel(core)
    await attach(model, core, "a.png")
    core.set(hold: true)
    let send = Task { await model.sendPrompt() }
    await core.waitHeld(1)
    #expect(!model.canSendPrompt)
    let upload = model.attach(name: "b.txt") { _ in Data([1]) }
    await core.waitHeld(2)
    model.draft = "next"
    core.release()
    await send.value
    await upload?.value
    #expect(core.snapshot.prompts == ["/Users/me/Library/Caches/dev.rbstp.collied/attachments/a.png/a.png"])
    #expect(model.draft == "next")
    #expect(model.attachments.map(\.name) == ["b.txt"])
}

@MainActor
@Test func secondSendWhileSendingIsIgnored() async {
    let core = FakeCore()
    let model = agentModel(core)
    model.draft = "one"
    core.set(hold: true)
    let first = Task { await model.sendPrompt() }
    await core.waitHeld(1)
    await model.sendPrompt()
    core.release()
    await first.value
    #expect(core.snapshot.prompts == ["one"])
}

@MainActor
@Test func blockedPromptKeepsDraftAndExplainsApprovals() async {
    let core = FakeCore()
    core.set(error: .AgentBlocked)
    let model = agentModel(core)
    model.draft = "continue"
    await model.sendPrompt()
    #expect(model.draft == "continue")
    #expect(!model.sendingPrompt)
    #expect(model.promptError?.contains("Approvals") == true)
    #expect(model.promptError?.contains("Phase") == false)

    core.set(error: .AgentNotReady)
    await model.sendPrompt()
    #expect(model.promptError == CoreError.AgentNotReady.description)
}

@Test func keyStripIsTheAllowlistInOrder() {
    #expect(AgentKey.strip == [.esc, .enter, .left, .up, .down, .right, .tab, .shiftTab, .ctrlC])
    #expect(AgentKey.strip.map(\.symbol) == ["esc", "⏎", "←", "↑", "↓", "→", "⇥", "⇧⇥", "^C"])
    let names = AgentKey.strip.map(\.accessibilityName)
    #expect(Set(names).count == names.count)
    #expect(names.allSatisfy { !$0.isEmpty })
}

@MainActor
@Test func eachStripKeySendsItsCoreKey() async {
    let core = FakeCore()
    let model = agentModel(core)
    for key in AgentKey.strip {
        await model.tap(key)?.value
    }
    #expect(core.snapshot.keys == AgentKey.strip.map { [$0] })
    #expect(model.keyTaps == AgentKey.strip.count)
}

@MainActor
@Test func tapsDuringASendAreBatchedInOrder() async {
    let core = FakeCore()
    let model = agentModel(core)
    core.set(hold: true)
    let drain = model.tap(.down)
    #expect(drain != nil)
    await core.waitHeld(1)
    #expect(model.tap(.down) == nil)
    #expect(model.tap(.enter) == nil)
    core.release()
    await drain?.value
    #expect(core.snapshot.keys == [[.down], [.down, .enter]])
}

@MainActor
@Test func failedKeysDropTheQueueAndShowTheError() async {
    let core = FakeCore()
    core.set(error: .AgentBlocked)
    let model = agentModel(core)
    await model.tap(.y)?.value
    #expect(model.notice?.contains("approval") == true)
}

@Test func closeNeedsTwoConfirmations() {
    var close = CloseConfirmation()
    #expect(close.confirm() == nil)
    close.advance()
    #expect(close.step == .idle)

    close.begin(.workspace(id: "w1"))
    #expect(close.step == .first(.workspace(id: "w1")))
    #expect(close.confirm() == nil)
    close.advance()
    #expect(close.step == .second(.workspace(id: "w1")))
    #expect(close.confirm() == .workspace(id: "w1"))
    #expect(close.step == .idle)
    #expect(close.confirm() == nil)

    close.begin(.pane)
    close.cancel()
    close.advance()
    #expect(close.confirm() == nil)

    close.begin(.pane)
    close.advance()
    close.cancel()
    #expect(close.confirm() == nil)
}

@MainActor
@Test func closeCallsCoreOnlyAfterBothSteps() async {
    let core = FakeCore()
    let model = agentModel(core)
    await model.performClose()
    model.close.begin(.pane)
    await model.performClose()
    #expect(core.snapshot.closes.isEmpty)
    #expect(!model.closed)

    model.close.advance()
    await model.performClose()
    #expect(core.snapshot.closes == ["pane term_1 confirm=true"])
    #expect(model.closed)

    let other = agentModel(core)
    other.close.begin(.workspace(id: "w9"))
    other.close.advance()
    await other.performClose()
    #expect(core.snapshot.closes.last == "workspace w9 confirm=true")
}

@MainActor
@Test func listCloseGoesThroughBothConfirmationSteps() async {
    let core = FakeCore()
    let model = FlockModel()
    let route = AgentRoute(machineId: "m1", terminalId: "term_2")
    #expect(await model.performClose(core: core) == false)

    model.beginClose(.pane, route: route)
    #expect(model.close.step == .first(.pane))
    #expect(await model.performClose(core: core) == false)
    #expect(core.snapshot.closes.isEmpty)
    model.close.advance()
    #expect(await model.performClose(core: core))
    #expect(core.snapshot.closes == ["pane term_2 confirm=true"])
    #expect(model.close.step == .idle)
    #expect(await model.performClose(core: core) == false)

    model.beginClose(.workspace(id: "w7"), route: route)
    model.close.cancel()
    model.close.advance()
    #expect(await model.performClose(core: core) == false)
    #expect(core.snapshot.closes.count == 1)

    core.set(error: .AgentBlocked)
    model.beginClose(.workspace(id: "w7"), route: route)
    model.close.advance()
    #expect(await model.performClose(core: core) == false)
    #expect(core.snapshot.closes.last == "workspace w7 confirm=true")
    #expect(model.closeNotice?.contains("approval") == true)
}

@MainActor
@Test func newTaskUsesTheDefaultAgentAndNeedsAnAbsoluteFolder() async {
    let core = FakeCore()
    let machine = Machine(id: "m1", label: "Mac", host: "mac.ts.net", port: 8457, nodeId: "n1")
    let model = NewTaskModel(core: core, machines: [machine])
    await model.loadOptions()
    #expect(model.agent == "codex")
    model.prompt = "add tests"
    model.cwd = "relative/path"
    #expect(!model.canStart)
    model.cwd = "/Users/me/app"
    #expect(model.canStart)
    #expect(await model.start() == nil)
    #expect(model.phase == .editing)
    #expect(model.error == CoreError.NotImplemented.description)
}

@MainActor
@Test func cancelledNewTaskNeverNavigates() async {
    let core = FakeCore()
    core.state.withLock { $0.started = TaskStarted(workspaceId: "w1", terminalId: "term_new") }
    let machine = Machine(id: "m1", label: "Mac", host: "mac.ts.net", port: 8457, nodeId: "n1")

    let model = NewTaskModel(core: core, machines: [machine])
    await model.loadOptions()
    model.cwd = "/Users/me/app"
    model.prompt = "add tests"
    #expect(await model.start() == AgentRoute(machineId: "m1", terminalId: "term_new"))

    let cancelled = NewTaskModel(core: core, machines: [machine])
    await cancelled.loadOptions()
    cancelled.cwd = "/Users/me/app"
    cancelled.prompt = "add tests"
    core.set(hold: true)
    let start = Task { await cancelled.start() }
    await core.waitHeld(1)
    cancelled.cancel()
    core.release()
    #expect(await start.value == nil)
    #expect(await cancelled.start() == nil)
}

@MainActor
@Test func wrapLinesIsOnByDefaultAndRememberedOnThisDevice() throws {
    let dir = FileManager.default.temporaryDirectory.appending(path: UUID().uuidString, directoryHint: .isDirectory)
    try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: dir) }
    let file = dir.appending(path: "prefs.json")
    let route = AgentRoute(machineId: "m1", terminalId: "term_1")

    let model = AgentModel(core: FakeCore(), route: route, prefsFile: file)
    #expect(model.wrapLines)
    model.wrapLines = false
    #expect(!AgentModel(core: FakeCore(), route: route, prefsFile: file).wrapLines)

    try Data("not json".utf8).write(to: file)
    #expect(AgentModel(core: FakeCore(), route: route, prefsFile: file).wrapLines)
}
