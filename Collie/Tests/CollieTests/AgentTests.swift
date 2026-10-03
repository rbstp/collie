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
        throw CoreError.NotImplemented
    }
    func flock(machineId: String) async throws -> MachineFlock { throw CoreError.MachineNotFound }
}

@MainActor
private func agentModel(_ core: FakeCore) -> AgentModel {
    AgentModel(core: core, route: AgentRoute(machineId: "m1", terminalId: "term_1"))
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
    #expect(model.promptError?.contains("Phase 3") == true)

    core.set(error: .AgentNotReady)
    await model.sendPrompt()
    #expect(model.promptError == CoreError.AgentNotReady.description)
}

@Test func keyStripIsTheAllowlistInOrder() {
    #expect(AgentKey.strip == [.esc, .enter, .up, .down, .tab, .shiftTab, .ctrlC, .y, .n])
    #expect(AgentKey.strip.map(\.symbol) == ["esc", "⏎", "↑", "↓", "⇥", "⇧⇥", "^C", "y", "n"])
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
