import CollieCore
import Foundation
import Synchronization
import Testing

@testable import Collie

final class FakeCore: AgentCore {
    struct State {
        var prompts: [String] = []
        var expectedDrafts: [String?] = []
        var kind: String?
        var macDraft: String?
        var draftReads = 0
        var keys: [[AgentKey]] = []
        var typed: [String] = []
        var closes: [String] = []
        var stars: [String] = []
        var scrolls: [String] = []
        var answers: [UInt8] = []
        var answerLabels: [String] = []
        var slashes: [String] = []
        var slashExpected: [String?] = []
        var slashErrors: [CoreError] = []
        var hold = false
        var held: [CheckedContinuation<Void, Never>] = []
        var error: CoreError?
        var options = TaskOptions(agents: ["claude", "codex"], defaultAgent: "codex", recentCwds: ["/Users/me/app"], roots: ["/Users/me"])
        var folders = TaskFolders(path: "/Users/me/git", folders: ["app", "collie", "Cobalt", "docs"], truncated: false)
        var folderPaths: [String] = []
        var taskNews: [String] = []
        var taskPrompts: [String] = []
        var started: TaskStarted?
        var uploads: [String] = []
        var cancelledUploads: [String] = []
        var maxAttachmentBytes: UInt64 = 20 * 1024 * 1024
        var uploadPath = "/Users/me/Library/Caches/dev.rbstp.collied/attachments/0123456789abcdef/notes.txt"
        var endedActivities: [String] = []
        var watches: [String?] = []
        var reads: [String] = []
        var readLines: [UInt16?] = []
        var readError: CoreError?
        var output: TerminalSnapshot?
        var outputRevision: UInt64 = 1
        var status: AgentState = .idle
        var lastPrompt: String?
        var depths: [UInt16?] = []
        var shell: TerminalSummary?
        var shellLocked = true
        var terminalsEnabled = true
        var challenges = 0
        var grants: [Data] = []
        var terminalError: CoreError?
        var shellWatches: [String] = []
        var commands: [String] = []
        var terminalKeys: [[AgentKey]] = []
        var machines: [Machine] = []
        var cachedFlock: MachineFlock?
        var flocks = 0
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

    func agentView(machineId: String, terminalId: String, afterRevision: UInt64) -> AgentView? {
        let s = state.withLock { $0 }
        if s.shell != nil || !s.terminalsEnabled {
            let agent = s.kind.map {
                AgentSummary(
                    terminalId: terminalId, workspaceId: "w1", kind: $0, name: nil, title: nil,
                    status: .idle, statusSinceMs: 0, cwd: nil, lastLine: nil
                )
            }
            return AgentView(
                link: .connected, lastError: nil, agent: agent, output: nil, outputRevision: 0,
                terminal: agent == nil ? s.shell : nil, terminalLocked: s.shellLocked, terminalsEnabled: s.terminalsEnabled
            )
        }
        if let output = state.withLock({ $0.output }) {
            let agent = s.kind.map {
                AgentSummary(
                    terminalId: terminalId, workspaceId: "w1", kind: $0, name: nil, title: nil,
                    status: s.status, statusSinceMs: 0, cwd: nil, lastLine: nil
                )
            }
            return AgentView(
                link: .connected, lastError: nil, agent: agent, output: output.terminalId == terminalId ? output : nil,
                outputRevision: s.outputRevision
            )
        }
        guard let kind = s.kind else { return nil }
        let agent = AgentSummary(
            terminalId: terminalId, workspaceId: "w1", kind: kind, name: nil, title: nil,
            status: s.status, statusSinceMs: 0, cwd: nil, lastLine: nil, lastPrompt: s.lastPrompt
        )
        return AgentView(link: .connected, lastError: nil, agent: agent, output: nil, outputRevision: 0)
    }
    func watchAgent(machineId: String, terminalId: String?, lines: UInt16) async throws {
        state.withLock {
            $0.watches.append(terminalId)
            $0.depths.append(lines)
        }
    }
    func agentRead(machineId: String, terminalId: String, source: TerminalSource, lines: UInt16?) async throws -> TerminalSnapshot {
        let (read, error) = state.withLock { s in
            s.reads.append(terminalId)
            s.readLines.append(lines)
            s.depths.append(lines)
            return (s.reads.count, s.readError)
        }
        if let error { throw error }
        return TerminalSnapshot(terminalId: terminalId, source: source, ansi: "read \(read)", truncated: false)
    }
    func agentDraft(machineId: String, terminalId: String) async throws -> String? {
        state.withLock { s in
            s.draftReads += 1
            return s.macDraft
        }
    }
    func prompt(machineId: String, terminalId: String, text: String, expectedDraft: String?) async throws {
        try await call {
            $0.prompts.append(text)
            $0.expectedDrafts.append(expectedDraft)
        }
    }
    func sendKeys(machineId: String, terminalId: String, keys: [AgentKey]) async throws {
        try await call { $0.keys.append(keys) }
    }
    func typeText(machineId: String, terminalId: String, text: String) async throws {
        try await call { $0.typed.append(text) }
    }
    func focus(machineId: String, terminalId: String) async throws {}
    func scrollBottom(machineId: String, terminalId: String) async throws {
        try await call { $0.scrolls.append(terminalId) }
    }
    func answerNotice(machineId: String, terminalId: String, digit: UInt8, label: String) async throws {
        try await call {
            $0.answers.append(digit)
            $0.answerLabels.append(label)
        }
    }
    func slashDraft(machineId: String, terminalId: String, command: String, expectedDraft: String?) async throws {
        try await call {
            $0.slashes.append(command)
            $0.slashExpected.append(expectedDraft)
        }
        let error = state.withLock { $0.slashErrors.isEmpty ? nil : $0.slashErrors.removeFirst() }
        if let error { throw error }
    }
    func star(machineId: String, terminalId: String, starred: Bool) async throws {
        try await call { $0.stars.append("\(machineId) \(terminalId) \(starred)") }
    }
    func closeWorkspace(machineId: String, workspaceId: String, confirm: Bool) async throws {
        try await call { $0.closes.append("workspace \(workspaceId) confirm=\(confirm)") }
    }
    func closePane(machineId: String, terminalId: String, confirm: Bool) async throws {
        try await call { $0.closes.append("pane \(terminalId) confirm=\(confirm)") }
    }
    func taskOptions(machineId: String) async throws -> TaskOptions {
        try await call { _ in }
        return state.withLock { $0.options }
    }
    func taskFolders(machineId: String, path: String) async throws -> TaskFolders {
        try await call { $0.folderPaths.append(path) }
        return state.withLock { $0.folders }
    }
    func taskNew(machineId: String, cwd: String, agent: String, prompt: String, label: String?, newFolder: String?) async throws -> TaskStarted {
        try await call {
            $0.taskNews.append("\(cwd) \(newFolder ?? "-")")
            $0.taskPrompts.append(prompt)
        }
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
    func terminalChallenge(machineId: String, terminalId: String) async throws -> Data {
        let error = state.withLock { s in
            s.challenges += 1
            return s.terminalError
        }
        if let error { throw error }
        return Data("challenge \(terminalId)".utf8)
    }
    func terminalGrant(machineId: String, terminalId: String, signature: Data) async throws {
        state.withLock {
            $0.grants.append(signature)
            $0.shellLocked = false
        }
    }
    func watchTerminal(machineId: String, terminalId: String, lines: UInt16) async throws {
        state.withLock { $0.shellWatches.append(terminalId) }
    }
    func terminalRun(machineId: String, terminalId: String, text: String) async throws {
        try await call { $0.commands.append(text) }
    }
    func terminalSendKeys(machineId: String, terminalId: String, keys: [AgentKey]) async throws {
        try await call { $0.terminalKeys.append(keys) }
    }
    func flock(machineId: String) async throws -> MachineFlock {
        let started = state.withLock { s in
            s.flocks += 1
            return s.started
        }
        guard let started else { throw CoreError.MachineNotFound }
        let machine = Machine(id: machineId, label: "Mac", host: "mac.ts.net", port: 8457, nodeId: "n1", kind: .mac, key: "")
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
@Test func aScreenReplacedAfterItsSuccessorWatchedLeavesTheWatch() async {
    let core = FakeCore()
    let first = AgentModel(core: core, route: AgentRoute(machineId: "replaced", terminalId: "term_1"), prefsFile: nil)
    let second = AgentModel(core: core, route: AgentRoute(machineId: "replaced", terminalId: "term_2"), prefsFile: nil)
    func waitWatches(_ count: Int) async {
        while core.snapshot.watches.count < count {
            try? await Task.sleep(for: .milliseconds(5))
        }
    }
    let firstRun = Task { await first.run() }
    await waitWatches(1)
    let secondRun = Task { await second.run() }
    await waitWatches(2)
    firstRun.cancel()
    await firstRun.value
    secondRun.cancel()
    await secondRun.value
    await waitWatches(3)
    try? await Task.sleep(for: .milliseconds(50))
    #expect(core.snapshot.watches == ["term_1", "term_2", nil])
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
        await model.attach(name: name) { _ in png(width: 1, height: 1) }?.value
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

    core.set(error: .AgentNotReady)
    await model.sendPrompt()
    #expect(model.promptError == CoreError.AgentNotReady.description)
}

@MainActor
private func openedAgent(_ core: FakeCore, kind: String = "claude", macDraft: String?) -> AgentModel {
    core.state.withLock {
        $0.kind = kind
        $0.macDraft = macDraft
    }
    let model = agentModel(core)
    model.poll()
    return model
}

@MainActor
@Test func macDraftFillsAnEmptyPromptFieldOnOpen() async {
    let core = FakeCore()
    let model = openedAgent(core, macDraft: "one\ntwo")
    await model.loadMacDraft()
    #expect(model.draft == "one\ntwo")
    #expect(model.macDraft == "one\ntwo")

    await model.sendPrompt()
    #expect(core.snapshot.prompts == ["one\ntwo"])
    #expect(core.snapshot.expectedDrafts == ["one\ntwo"])
    #expect(model.macDraft == "")
    #expect(model.draft.isEmpty)
}

@MainActor
@Test func macDraftIsReadOnceTheAgentIsKnown() async {
    let core = FakeCore()
    core.state.withLock {
        $0.kind = nil
        $0.macDraft = "from the mac"
    }
    let model = agentModel(core)
    let run = Task { await model.run() }
    try? await Task.sleep(for: .milliseconds(300))
    #expect(core.snapshot.draftReads == 0)
    core.state.withLock { $0.kind = "claude" }
    for _ in 0..<400 where model.macDraft == nil {
        try? await Task.sleep(for: .milliseconds(5))
    }
    run.cancel()
    await run.value
    #expect(core.snapshot.draftReads == 1)
    #expect(model.draft == "from the mac")
}

@MainActor
@Test func macDraftNeverOverwritesThePhoneDraft() async {
    let core = FakeCore()
    let typed = openedAgent(core, macDraft: "from the mac")
    typed.draft = "from the phone"
    await typed.loadMacDraft()
    #expect(typed.draft == "from the phone")
    #expect(typed.macDraft == nil)

    let attached = openedAgent(core, macDraft: "from the mac")
    await attach(attached, core, "a.png")
    await attached.loadMacDraft()
    #expect(attached.draft.isEmpty)
    #expect(attached.macDraft == nil)
}

@MainActor
@Test func macDraftIsOnlyReadForClaude() async {
    let core = FakeCore()
    let codex = openedAgent(core, kind: "codex", macDraft: "typed")
    await codex.loadMacDraft()
    #expect(codex.draft.isEmpty)
    #expect(core.snapshot.draftReads == 0)

    let empty = openedAgent(core, macDraft: "")
    await empty.loadMacDraft()
    #expect(empty.macDraft == "")
    #expect(empty.draft.isEmpty)

    let unknown = openedAgent(core, macDraft: nil)
    await unknown.loadMacDraft()
    #expect(unknown.macDraft == nil)
    unknown.draft = "go"
    await unknown.sendPrompt()
    unknown.draft = "again"
    await unknown.sendPrompt()
    #expect(unknown.macDraft == nil)
    #expect(core.snapshot.expectedDrafts == [nil, nil])
}

@MainActor
@Test func changedMacDraftIsShownThenReplacedOnTheNextSend() async {
    let core = FakeCore()
    let model = openedAgent(core, macDraft: "")
    await model.loadMacDraft()
    model.draft = "phone text"
    let long = String(repeating: "x", count: 100)
    core.set(error: .DraftChanged(current: long))
    await model.sendPrompt()
    #expect(model.draft == "phone text")
    #expect(model.macDraft == long)
    let shown = String(repeating: "x", count: 80) + "…"
    #expect(model.promptError == "The agent's input box has unsent text: “\(shown)”. Send again to replace it.")

    core.set(error: nil)
    await model.sendPrompt()
    #expect(core.snapshot.prompts == ["phone text", "phone text"])
    #expect(core.snapshot.expectedDrafts == ["", long])
    #expect(model.promptError == nil)
    #expect(model.macDraft == "")
    #expect(model.draft.isEmpty)
}

@MainActor
@Test func uncleanedMacDraftKeepsTheDraftAndAttachments() async {
    let core = FakeCore()
    let model = openedAgent(core, macDraft: "mac text")
    await model.loadMacDraft()
    await attach(model, core, "a.png")
    core.set(error: .DraftNotCleared)
    await model.sendPrompt()
    #expect(model.draft == "mac text")
    #expect(model.attachments.map(\.name) == ["a.png"])
    #expect(model.macDraft == "mac text")
    #expect(model.promptError == CoreError.DraftNotCleared.description)
}

@MainActor
private func macSends(_ core: FakeCore, _ model: AgentModel, prompt: String) async {
    core.state.withLock {
        $0.macDraft = ""
        $0.lastPrompt = prompt
    }
    model.poll()
    await model.sentMacDraftCheck?.value
}

@MainActor
@Test func aPromptSentFromTheMacClearsTheDraftLoadedFromIt() async {
    let core = FakeCore()
    let model = openedAgent(core, macDraft: "test")
    await model.loadMacDraft()
    await attach(model, core, "a.png")
    await macSends(core, model, prompt: "test")
    #expect(model.draft.isEmpty)
    #expect(model.macDraft == "")
    #expect(model.attachments.map(\.name) == ["a.png"])
}

@MainActor
@Test func aPromptSentFromTheMacClearsTheLoadedDraftEvenAfterTheMacTextChanged() async {
    let core = FakeCore()
    let model = openedAgent(core, macDraft: "test")
    await model.loadMacDraft()
    core.state.withLock { $0.macDraft = "test2" }
    model.poll()
    await macSends(core, model, prompt: "test2")
    #expect(model.draft.isEmpty)
}

@MainActor
@Test func aPromptQueuedOnTheMacWhileWorkingClearsTheLoadedDraft() async {
    let core = FakeCore()
    core.state.withLock {
        $0.status = .working
        $0.lastPrompt = "earlier"
    }
    let model = openedAgent(core, macDraft: "next")
    await model.loadMacDraft()
    await macSends(core, model, prompt: "next")
    #expect(model.draft.isEmpty)
}

@MainActor
@Test func aDraftEditedOnThePhoneStaysWhenTheMacSends() async {
    let core = FakeCore()
    let model = openedAgent(core, macDraft: "test")
    await model.loadMacDraft()
    model.typed("test from the phone")
    await macSends(core, model, prompt: "test")
    #expect(model.draft == "test from the phone")

    let other = FakeCore()
    let pasted = openedAgent(other, macDraft: "test")
    await pasted.loadMacDraft()
    pasted.paste(" more")
    await macSends(other, pasted, prompt: "test")
    #expect(pasted.draft == "test more")
}

@MainActor
@Test func macTextDeletedWithoutSendingKeepsTheLoadedDraft() async {
    let core = FakeCore()
    let model = openedAgent(core, macDraft: "test")
    await model.loadMacDraft()
    core.state.withLock { $0.macDraft = "" }
    model.poll()
    await model.sentMacDraftCheck?.value
    #expect(model.draft == "test")

    core.state.withLock { $0.status = .blocked }
    model.poll()
    core.state.withLock { $0.status = .working }
    model.poll()
    await model.sentMacDraftCheck?.value
    #expect(model.draft == "test")
}

@MainActor
@Test func aTurnWithoutANewPromptKeepsTheLoadedDraft() async {
    let core = FakeCore()
    core.state.withLock { $0.lastPrompt = "earlier" }
    let model = openedAgent(core, macDraft: "test")
    await model.loadMacDraft()
    core.state.withLock {
        $0.macDraft = ""
        $0.status = .working
    }
    model.poll()
    core.state.withLock { $0.lastPrompt = nil }
    model.poll()
    core.state.withLock { $0.lastPrompt = "earlier" }
    model.poll()
    await model.sentMacDraftCheck?.value
    #expect(model.draft == "test")
    #expect(model.sentMacDraftCheck == nil)
}

@Test func keyStripIsTheAllowlistInOrder() {
    #expect(AgentKey.strip == [.esc, .left, .up, .down, .right, .tab, .shiftTab, .enter, .ctrlEnter])
    #expect(AgentKey.strip.map(\.symbol) == ["esc", "←", "↑", "↓", "→", "⇥", "⇧⇥", "⏎", "⌃⏎"])
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
    await model.tap(.down)?.value
    #expect(model.notice?.contains("approval") == true)
}

@Test func closeNeedsAConfirmation() {
    var close = CloseConfirmation()
    #expect(close.confirm() == nil)
    close.advance()
    #expect(close.step == .idle)

    close.begin(.workspace(id: "w1"))
    #expect(close.step == .asking(.workspace(id: "w1")))
    #expect(close.confirm() == nil)
    close.advance()
    #expect(close.step == .confirmed(.workspace(id: "w1")))
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
@Test func closeCallsCoreOnlyAfterTheConfirmation() async {
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
@Test func listCloseGoesThroughTheConfirmation() async {
    let core = FakeCore()
    let model = FlockModel()
    let route = AgentRoute(machineId: "m1", terminalId: "term_2")
    #expect(await model.performClose(core: core) == false)

    model.beginClose(.pane, route: route)
    #expect(model.close.step == .asking(.pane))
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
@Test func theListReadsTheCacheBetweenSnapshotsAndRetriesAFailedRead() async {
    let core = FakeCore()
    let machine = Machine(id: "m1", label: "Mac", host: "mac.ts.net", port: 8457, nodeId: "n1", kind: .mac, key: "")
    func cached(_ title: String) -> MachineFlock {
        let agent = AgentSummary(
            terminalId: "term_1", workspaceId: "w1", kind: "claude", name: nil, title: title,
            status: .working, statusSinceMs: 0, cwd: nil, lastLine: nil
        )
        return MachineFlock(machine: machine, link: .connected, lastError: nil, details: nil, workspaces: [], agents: [agent], approvalsCount: 0)
    }
    core.state.withLock {
        $0.machines = [machine]
        $0.cachedFlock = cached("first")
    }
    let model = FlockModel()

    await model.refresh(core: core)
    #expect(core.snapshot.flocks == 1)
    #expect(model.entries.first?.error != nil)
    #expect(model.entries.first?.agents.first?.title == "first")

    core.state.withLock { $0.started = TaskStarted(workspaceId: "w1", terminalId: "term_2") }
    await model.refresh(core: core, snapshot: false)
    #expect(core.snapshot.flocks == 2)
    #expect(model.entries.first?.error == nil)

    core.state.withLock { $0.cachedFlock = cached("second") }
    await model.refresh(core: core, snapshot: false)
    #expect(core.snapshot.flocks == 2)
    #expect(model.entries.first?.agents.map(\.title) == ["second"])

    await model.refresh(core: core)
    #expect(core.snapshot.flocks == 3)

    core.state.withLock {
        $0.cachedFlock?.lastError = "herdr is not running on the machine"
        $0.started = nil
    }
    await model.refresh(core: core, snapshot: false)
    #expect(core.snapshot.flocks == 3)
    #expect(model.entries.first?.error == "herdr is not running on the machine")
    await model.refresh(core: core, snapshot: false)
    #expect(core.snapshot.flocks == 4)
}

@MainActor
@Test func theConnectingPollOnlyReadsTheCache() async {
    let core = FakeCore()
    let machine = Machine(id: "m1", label: "Mac", host: "mac.ts.net", port: 8457, nodeId: "n1", kind: .mac, key: "")
    let details = MachineDetails(name: "Mac", nodeId: "n1", herdrSession: "default")
    func flock(_ link: LinkPhase, details: MachineDetails? = nil, error: String? = nil) -> MachineFlock {
        MachineFlock(machine: machine, link: link, lastError: error, details: details, workspaces: [], agents: [], approvalsCount: 0)
    }
    core.state.withLock {
        $0.machines = [machine]
        $0.cachedFlock = flock(.offline)
    }
    let model = FlockModel()

    await model.refresh(core: core, snapshot: false, cacheOnly: true)
    #expect(core.snapshot.flocks == 0)
    #expect(model.entries.first?.connecting(nodeStarting: true) == true)
    core.state.withLock { $0.cachedFlock = flock(.connected) }
    await model.refresh(core: core, snapshot: false, cacheOnly: true)
    #expect(model.entries.first?.connecting(nodeStarting: true) == true, "connected, but its list has not landed yet")
    core.state.withLock { $0.cachedFlock = flock(.connected, details: details) }
    await model.refresh(core: core, snapshot: false, cacheOnly: true)
    #expect(model.entries.first?.connecting(nodeStarting: true) == false)
    #expect(core.snapshot.flocks == 0)

    core.state.withLock { $0.cachedFlock = flock(.connected, error: "herdr is not running on the machine") }
    await model.refresh(core: core, snapshot: false, cacheOnly: true)
    #expect(model.entries.first?.error != nil)
    #expect(model.entries.first?.connecting(nodeStarting: true) == false)
    await model.refresh(core: core, snapshot: false, cacheOnly: true)
    #expect(core.snapshot.flocks == 0, "a failed read is retried over the network by the 3 s refresh only")

    for (link, connecting) in [(LinkPhase.connecting, true), (.waiting, false), (.unavailable, false), (.stopped, false)] {
        #expect(MachineFlockEntry(machine: machine, flock: flock(link)).connecting(nodeStarting: true) == connecting)
    }
    #expect(MachineFlockEntry(machine: machine).connecting(nodeStarting: false))
    #expect(MachineFlockEntry(machine: machine, flock: flock(.connecting)).connecting(nodeStarting: false))
    #expect(
        !MachineFlockEntry(machine: machine, flock: flock(.offline)).connecting(nodeStarting: false),
        "a node that never runs is not polled fast past the start window"
    )
    #expect(!MachineFlockEntry(machine: machine, flock: flock(.connecting), error: "machine not found").connecting(nodeStarting: true))
}

@MainActor
@Test func newTaskUsesTheDefaultAgentAndNeedsAnAbsoluteFolder() async {
    let core = FakeCore()
    let machine = Machine(id: "m1", label: "Mac", host: "mac.ts.net", port: 8457, nodeId: "n1", kind: .mac, key: "")
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
@Test func newTaskPrefersAConnectedMachineAndIgnoresALateErrorFromTheLastOne() async {
    let core = FakeCore()
    let mac = Machine(id: "m1", label: "Mac", host: "mac.ts.net", port: 8457, nodeId: "n1", kind: .mac, key: "")
    let linux = Machine(id: "m2", label: "omarchy", host: "omarchy.ts.net", port: 8457, nodeId: "n2", kind: .linux, key: "")
    #expect(NewTaskModel(core: core, machines: [mac, linux], preferredMachineId: "m2").machineId == "m2")
    #expect(NewTaskModel(core: core, machines: [mac, linux], preferredMachineId: "gone").machineId == "m1")
    #expect(NewTaskModel(core: core, machines: [mac, linux]).machineId == "m1")

    let model = NewTaskModel(core: core, machines: [mac, linux])
    core.set(hold: true)
    let first = Task { await model.loadOptions() }
    await core.waitHeld(1)
    model.machineId = "m2"
    core.state.withLock { $0.hold = false }
    await model.loadOptions()
    #expect(model.options?.defaultAgent == "codex")

    core.state.withLock { $0.error = .MachineNotFound }
    core.release()
    await first.value
    #expect(model.optionsError == nil)
    #expect(model.options?.defaultAgent == "codex")
}

private func prefsFile() throws -> URL {
    let dir = FileManager.default.temporaryDirectory.appending(path: UUID().uuidString, directoryHint: .isDirectory)
    try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
    return dir.appending(path: "prefs.json")
}

@MainActor
@Test func aNameInTheBaseFolderCompletesAndStartsThere() async throws {
    let core = FakeCore()
    core.state.withLock { $0.started = TaskStarted(workspaceId: "w1", terminalId: "term_new") }
    let file = try prefsFile()
    defer { try? FileManager.default.removeItem(at: file.deletingLastPathComponent()) }
    let mac = Machine(id: "m1", label: "Mac", host: "mac.ts.net", port: 8457, nodeId: "n1", kind: .mac, key: "")
    let other = Machine(id: "m2", label: "omarchy", host: "omarchy.ts.net", port: 8457, nodeId: "n2", kind: .linux, key: "")

    #expect(try await DevicePrefs.setTaskBase("/Users/me/git/", machineId: "m1", core: core, in: file) == "/Users/me/git")
    #expect(DevicePrefs.load(from: file).taskBases == ["m1": "/Users/me/git"])
    core.set(error: .InvalidInput(field: nil, message: "cwd is outside the allowed roots"))
    await #expect(throws: CoreError.self) {
        try await DevicePrefs.setTaskBase("/etc", machineId: "m2", core: core, in: file)
    }
    #expect(DevicePrefs.load(from: file).taskBases == ["m1": "/Users/me/git"], "a refused folder is never kept")
    core.set()

    let model = NewTaskModel(core: core, machines: [mac, other], prefsFile: file)
    await model.loadOptions()
    #expect(model.base == "/Users/me/git")
    #expect(model.folders?.folders.count == 4)
    #expect(core.snapshot.folderPaths == ["/Users/me/git/", "/etc", "/Users/me/git"])
    model.prompt = "add tests"
    model.cwd = "co"
    #expect(model.completions == ["collie", "Cobalt"])
    #expect(model.folder == "/Users/me/git/co")
    model.cwd = "collie"
    #expect(model.completions.isEmpty)
    #expect(model.canStart)
    model.cwd = "/Users/me/app"
    #expect(model.folder == "/Users/me/app")
    model.cwd = "collie"
    #expect(await model.start() == AgentRoute(machineId: "m1", terminalId: "term_new"))
    #expect(core.snapshot.taskNews == ["/Users/me/git/collie -"])

    let noBase = NewTaskModel(core: core, machines: [mac, other], preferredMachineId: "m2", prefsFile: file)
    await noBase.loadOptions()
    #expect(noBase.base == nil && noBase.folders == nil)
    noBase.prompt = "add tests"
    noBase.cwd = "collie"
    #expect(noBase.folder == nil)
    #expect(!noBase.canStart)
    #expect(core.snapshot.folderPaths.count == 3, "no listing without a base folder")

    DevicePrefs.forgetTaskBase(machineId: "m1", in: file)
    #expect(DevicePrefs.load(from: file).taskBases.isEmpty)
}

@MainActor
@Test func aNewFolderSendsItsParentAndName() async throws {
    let core = FakeCore()
    core.state.withLock { $0.started = TaskStarted(workspaceId: "w1", terminalId: "term_new") }
    let file = try prefsFile()
    defer { try? FileManager.default.removeItem(at: file.deletingLastPathComponent()) }
    let mac = Machine(id: "m1", label: "Mac", host: "mac.ts.net", port: 8457, nodeId: "n1", kind: .mac, key: "")

    let model = NewTaskModel(core: core, machines: [mac], prefsFile: file)
    await model.loadOptions()
    model.prompt = "set it up"
    model.newFolder = true
    #expect(model.newFolderParent == "/Users/me", "the first root without a base folder")
    for bad in ["", ".hidden", "a/b", "  "] {
        model.folderName = bad
        #expect(!model.canStart, "\(bad)")
    }
    model.folderName = " fresh "
    #expect(model.canStart)
    #expect(await model.start() != nil)

    core.state.withLock { $0.options.roots = ["/Users/me", "/Volumes/work"] }
    let twoRoots = NewTaskModel(core: core, machines: [mac], prefsFile: file)
    await twoRoots.loadOptions()
    twoRoots.prompt = "set it up"
    twoRoots.newFolder = true
    twoRoots.folderName = "other"
    #expect(twoRoots.newFolderParent == "/Users/me")
    twoRoots.newFolderRoot = "/Volumes/work"
    #expect(await twoRoots.start() != nil)

    DevicePrefs.update(in: file) { $0.taskBases["m1"] = "/Users/me/git" }
    let based = NewTaskModel(core: core, machines: [mac], prefsFile: file)
    await based.loadOptions()
    based.prompt = "set it up"
    based.newFolder = true
    based.folderName = "COLLIE"
    #expect(await based.start() != nil)
    #expect(core.snapshot.taskNews == ["/Users/me fresh", "/Volumes/work other", "/Users/me/git COLLIE"])

    core.state.withLock { $0.started = nil }
    let failed = NewTaskModel(core: core, machines: [mac], prefsFile: file)
    await failed.loadOptions()
    failed.prompt = "set it up"
    failed.newFolder = true
    failed.folderName = "brand-new"
    #expect(await failed.start() == nil)
    #expect(failed.error != nil && failed.newFolder, "a name that was not created keeps New folder on")
    failed.folderName = "collie"
    #expect(await failed.start() == nil)
    #expect(!failed.newFolder && failed.cwd == "/Users/me/git/collie", "the folder left in place is the next start's folder")
    #expect(failed.canStart)
}

@MainActor
@Test func newTaskUploadsFilesIntoItsInitialPromptAndClearsThemOnMachineChange() async {
    let core = FakeCore()
    core.state.withLock { $0.started = TaskStarted(workspaceId: "w1", terminalId: "term_new") }
    let mac = Machine(id: "m1", label: "Mac", host: "mac.ts.net", port: 8457, nodeId: "n1", kind: .mac, key: "")
    let other = Machine(id: "m2", label: "Other", host: "other.ts.net", port: 8457, nodeId: "n2", kind: .mac, key: "")
    let model = NewTaskModel(core: core, machines: [mac, other], prefsFile: nil)
    await model.loadOptions()
    model.cwd = "/Users/me/app"
    #expect(!model.canStart)

    await model.attach([PendingAttachment(name: "notes.txt") { _ in Data("notes".utf8) }])?.value
    #expect(model.attachments.count == 1)
    #expect(model.canStart)
    model.prompt = "review this"
    #expect(await model.start() != nil)
    #expect(core.snapshot.taskPrompts == ["/Users/me/Library/Caches/dev.rbstp.collied/attachments/0123456789abcdef/notes.txt review this"])

    model.machineId = "m2"
    #expect(model.attachments.isEmpty)
    #expect(core.snapshot.cancelledUploads.isEmpty)
}

@MainActor
@Test func newTaskRejectsAnOversizedAttachmentWithoutStarting() async {
    let core = FakeCore()
    core.state.withLock { $0.maxAttachmentBytes = 2 }
    let mac = Machine(id: "m1", label: "Mac", host: "mac.ts.net", port: 8457, nodeId: "n1", kind: .mac, key: "")
    let model = NewTaskModel(core: core, machines: [mac], prefsFile: nil)
    await model.loadOptions()
    model.cwd = "/Users/me/app"
    await model.attach([PendingAttachment(name: "notes.txt") { _ in Data("notes".utf8) }])?.value
    #expect(model.attachments.isEmpty)
    #expect(model.attachmentError != nil)
    #expect(core.snapshot.uploads.isEmpty)
    #expect(!model.canStart)
}

@MainActor
@Test func newTaskCancelsAnUploadWhenItsMachineChanges() async {
    let core = FakeCore()
    let mac = Machine(id: "m1", label: "Mac", host: "mac.ts.net", port: 8457, nodeId: "n1", kind: .mac, key: "")
    let other = Machine(id: "m2", label: "Other", host: "other.ts.net", port: 8457, nodeId: "n2", kind: .mac, key: "")
    let model = NewTaskModel(core: core, machines: [mac, other], prefsFile: nil)
    await model.loadOptions()
    core.set(hold: true)
    let upload = model.attach([PendingAttachment(name: "notes.txt") { _ in Data("notes".utf8) }])
    await core.waitHeld(1)
    model.machineId = "m2"
    core.release()
    await upload?.value
    #expect(model.attachments.isEmpty)
    #expect(model.upload == nil)
    #expect(core.snapshot.cancelledUploads == ["m1"])
}

@MainActor
@Test func prefsSavedBeforeTaskBasesStillLoad() throws {
    let file = try prefsFile()
    defer { try? FileManager.default.removeItem(at: file.deletingLastPathComponent()) }
    try Data(#"{"wrapLines":false,"historyLines":500}"#.utf8).write(to: file)
    let prefs = DevicePrefs.load(from: file)
    #expect(prefs == DevicePrefs(wrapLines: false, historyLines: 500))
    #expect(prefs.taskBases.isEmpty)
    DevicePrefs.forgetTaskBase(machineId: "m1", in: file)
    #expect(DevicePrefs.load(from: file) == prefs)
}

@MainActor
@Test func cancelledNewTaskNeverNavigates() async {
    let core = FakeCore()
    core.state.withLock { $0.started = TaskStarted(workspaceId: "w1", terminalId: "term_new") }
    let machine = Machine(id: "m1", label: "Mac", host: "mac.ts.net", port: 8457, nodeId: "n1", kind: .mac, key: "")

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
@Test func wrappedLinesShowTheAgentsProseRejoined() {
    let core = FakeCore()
    core.state.withLock {
        $0.output = TerminalSnapshot(terminalId: "term_1", source: .recent, ansi: "⏺ a\n  b", truncated: false, reflowed: "⏺ a b")
    }
    let model = AgentModel(core: core, route: AgentRoute(machineId: "m1", terminalId: "term_1"), prefsFile: nil)
    model.poll()
    #expect(model.screen == "⏺ a b")
    model.wrapLines = false
    #expect(model.screen == "⏺ a\n  b")
    model.wrapLines = true
    core.state.withLock { $0.output = TerminalSnapshot(terminalId: "term_1", source: .recent, ansi: "$ ls", truncated: false) }
    model.poll()
    #expect(model.screen == "$ ls")
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

@MainActor
@Test func keepKeyboardIsOffByDefaultAndSavedBesideWrapLines() throws {
    let dir = FileManager.default.temporaryDirectory.appending(path: UUID().uuidString, directoryHint: .isDirectory)
    try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: dir) }
    let file = dir.appending(path: "prefs.json")
    let route = AgentRoute(machineId: "m1", terminalId: "term_1")

    try Data(#"{"wrapLines":false}"#.utf8).write(to: file)
    let model = AgentModel(core: FakeCore(), route: route, prefsFile: file)
    #expect(!model.wrapLines)
    #expect(!model.keepsKeyboard)

    var prefs = DevicePrefs.load(from: file)
    prefs.keepKeyboard = true
    prefs.save(to: file)
    #expect(model.keepsKeyboard)

    model.wrapLines = true
    #expect(DevicePrefs.load(from: file) == DevicePrefs(wrapLines: true, keepKeyboard: true))
}

@MainActor
@Test func terminalHistoryIs200LinesByDefaultAndRememberedOnThisDevice() async throws {
    let dir = FileManager.default.temporaryDirectory.appending(path: UUID().uuidString, directoryHint: .isDirectory)
    try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: dir) }
    let file = dir.appending(path: "prefs.json")
    let core = FakeCore()
    let model = AgentModel(core: core, route: AgentRoute(machineId: "history", terminalId: "term_1"), prefsFile: file)
    func runOnce(_ watches: Int) async {
        let run = Task { await model.run() }
        while core.snapshot.watches.count < watches {
            try? await Task.sleep(for: .milliseconds(5))
        }
        run.cancel()
        await run.value
    }

    try Data(#"{"wrapLines":false}"#.utf8).write(to: file)
    #expect(DevicePrefs.load(from: file).historyLines == 200)
    await runOnce(1)
    await model.refresh()

    var prefs = DevicePrefs.load(from: file)
    prefs.historyLines = 1000
    prefs.save(to: file)
    await runOnce(3)
    await model.refresh()
    while core.snapshot.watches.count < 4 {
        try? await Task.sleep(for: .milliseconds(5))
    }
    #expect(core.snapshot.watches == ["term_1", nil, "term_1", nil])
    #expect(core.snapshot.depths == [200, 200, 200, 1000, 1000, 1000])
    #expect(DevicePrefs.load(from: file) == DevicePrefs(wrapLines: false, historyLines: 1000))
}

@MainActor
@Test func gesturesAndTextSizeHaveDefaultsAndAreRememberedOnThisDevice() throws {
    let dir = FileManager.default.temporaryDirectory.appending(path: UUID().uuidString, directoryHint: .isDirectory)
    try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: dir) }
    let file = dir.appending(path: "prefs.json")
    let route = AgentRoute(machineId: "m1", terminalId: "term_1")

    try Data(#"{"wrapLines":false,"keepKeyboard":true}"#.utf8).write(to: file)
    let model = AgentModel(core: FakeCore(), route: route, prefsFile: file)
    #expect(
        model.gestures
            == TerminalGestures(doubleTap: .paste, tripleTap: .none, pinchResizesText: true, swipeSwitchesAgents: true)
    )
    #expect(model.fontSize == 11)

    model.fontSize = 14
    var prefs = DevicePrefs.load(from: file)
    prefs.gestures.doubleTap = .none
    prefs.save(to: file)
    let reopened = AgentModel(core: FakeCore(), route: route, prefsFile: file)
    #expect(reopened.fontSize == 14)
    #expect(reopened.gestures.doubleTap == .none)
    #expect(!reopened.wrapLines && reopened.keepsKeyboard)

    prefs.gestures.tripleTap = .escape
    prefs.save(to: file)
    reopened.reloadGestures()
    #expect(reopened.gestures.tripleTap == .escape)

    try Data(#"{"wrapLines":false,"gestures":{"doubleTap":"later"}}"#.utf8).write(to: file)
    #expect(DevicePrefs.load(from: file) == DevicePrefs(wrapLines: false))
}

@MainActor
@Test func pasteFillsThePromptFieldWithoutSending() async {
    let core = FakeCore()
    let model = agentModel(core)
    model.draft = "fix "
    model.paste("the build")
    model.paste(nil)
    #expect(model.draft == "fix the build")
    await Task.yield()
    #expect(core.snapshot.prompts.isEmpty && core.snapshot.typed.isEmpty && core.snapshot.keys.isEmpty)
}

@MainActor
@Test func blockedInputGatesKeysAndTypedAnswers() async {
    let core = FakeCore()
    let model = agentModel(core)
    #expect(model.acceptsKeys && !model.answering && model.blockedHint == nil)

    model.blocked = .terminal
    #expect(!model.acceptsKeys && !model.answering)
    #expect(model.blockedHint == nil)
    #expect(model.tap(.enter) == nil)

    model.blocked = .optionsOnly
    #expect(!model.acceptsKeys && !model.answering)
    #expect(model.blockedHint == "Choose an option above.")
    #expect(model.tap(.down) == nil)
    #expect(model.keyTaps == 0)
    model.draft = "go"
    await model.sendPrompt()
    #expect(core.snapshot.typed.isEmpty)
    #expect(core.snapshot.prompts == ["go"])

    model.blocked = .keys
    #expect(model.acceptsKeys && !model.answering)
    await model.tap(.down)?.value
    #expect(core.snapshot.keys == [[.down]])

    model.blocked = .keysAndText
    #expect(model.acceptsKeys && model.answering)
    #expect(model.blockedHint == "Choose an option above, use the arrow keys, or type an answer.")
}

@MainActor
@Test func aScrolledUpTranscriptOffersJumpToBottomInsteadOfKeys() async {
    let core = FakeCore()
    core.state.withLock {
        $0.kind = "claude"
        $0.output = TerminalSnapshot(terminalId: "term_1", source: .recent, ansi: "Jump to bottom ↓", truncated: false, jumpBanner: true)
    }
    let model = agentModel(core)
    model.poll()
    #expect(model.jumpBanner && !model.acceptsKeys)
    #expect(model.tap(.down) == nil)
    await model.jumpToBottom()
    #expect(core.snapshot.scrolls == ["term_1"])
    #expect(core.snapshot.keys.isEmpty)
    #expect(!model.jumpBanner, "hidden until a new screen shows the banner again")

    core.set(error: .AgentNotReady)
    await model.jumpToBottom()
    #expect(model.notice != nil && model.jumpBanner)
    core.set()

    core.state.withLock { $0.output = TerminalSnapshot(terminalId: "term_1", source: .recent, ansi: "❯ ", truncated: false) }
    model.poll()
    #expect(!model.jumpBanner && model.acceptsKeys)
}

@MainActor
@Test func onlyClaudeCodeOffersJumpToBottom() {
    let core = FakeCore()
    core.state.withLock {
        $0.kind = "codex"
        $0.output = TerminalSnapshot(terminalId: "term_1", source: .recent, ansi: "12 new messages ↓", truncated: false, jumpBanner: true)
    }
    let model = agentModel(core)
    model.poll()
    #expect(!model.jumpBanner && model.acceptsKeys)
}

@MainActor private let rating = [
    NoticeOption(digit: 1, label: "Bad"), NoticeOption(digit: 2, label: "Fine"),
    NoticeOption(digit: 3, label: "Good"), NoticeOption(digit: 0, label: "Dismiss"),
]

@MainActor
private func noticeShown(
    _ core: FakeCore, kind: String = "claude", jumpBanner: Bool = false, clock: FakeClock = FakeClock()
) -> AgentModel {
    core.state.withLock {
        $0.kind = kind
        $0.output = TerminalSnapshot(
            terminalId: "term_1", source: .recent, ansi: "● How is Claude doing this session? (optional)", truncated: false,
            jumpBanner: jumpBanner, notice: rating
        )
    }
    let model = AgentModel(core: core, route: AgentRoute(machineId: "m1", terminalId: "term_1"), prefsFile: nil) { clock.now }
    model.poll()
    clock.advance(.milliseconds(600))
    model.poll()
    return model
}

@MainActor
@Test func aNoticeOffersItsOptionsInScreenOrder() async {
    let core = FakeCore()
    let clock = FakeClock()
    let model = noticeShown(core, clock: clock)
    #expect(model.noticeOptions.map(\.label) == ["Bad", "Fine", "Good", "Dismiss"])
    #expect(model.acceptsKeys)
    await model.answerNotice(model.noticeOptions[2])
    #expect(core.snapshot.answers == [3] && core.snapshot.answerLabels == ["Good"])
    #expect(core.snapshot.prompts.isEmpty && core.snapshot.keys.isEmpty)
    #expect(model.noticeOptions.isEmpty, "hidden until a new screen shows the notice again")

    model.poll()
    clock.advance(.milliseconds(600))
    model.poll()
    core.set(error: .AgentNotReady)
    await model.answerNotice(model.noticeOptions[3])
    #expect(model.notice != nil && model.noticeOptions == rating)

    core.set(hold: true, error: .AgentNotReady)
    let refused = Task { await model.answerNotice(model.noticeOptions[0]) }
    await core.waitHeld(1)
    core.state.withLock {
        $0.output = TerminalSnapshot(terminalId: "term_1", source: .recent, ansi: "❯ ", truncated: false)
        $0.outputRevision = 2
    }
    model.poll()
    core.release()
    await refused.value
    #expect(model.notice != nil && model.noticeOptions.isEmpty, "a newer screen without the notice wins over the restore")
    core.set()
}

@MainActor
@Test func noticeOptionsWaitUntilClaudeCodeTakesTheDigit() {
    let core = FakeCore()
    core.state.withLock {
        $0.kind = "claude"
        $0.output = TerminalSnapshot(
            terminalId: "term_1", source: .recent, ansi: "● How is Claude doing this session? (optional)", truncated: false,
            notice: rating
        )
    }
    let clock = FakeClock()
    let model = AgentModel(core: core, route: AgentRoute(machineId: "m1", terminalId: "term_1"), prefsFile: nil) { clock.now }
    model.poll()
    #expect(model.noticeOptions.isEmpty)
    clock.advance(.milliseconds(599))
    model.poll()
    #expect(model.noticeOptions.isEmpty)
    clock.advance(.milliseconds(1))
    model.poll()
    #expect(model.noticeOptions == rating)

    let headsUp = [NoticeOption(digit: 1, label: "Learn more"), NoticeOption(digit: 0, label: "Dismiss")]
    core.state.withLock {
        $0.output = TerminalSnapshot(terminalId: "term_1", source: .recent, ansi: "✦ Heads up", truncated: false, notice: headsUp)
        $0.outputRevision = 2
    }
    model.poll()
    #expect(model.noticeOptions.isEmpty, "another notice waits again")
    clock.advance(.milliseconds(600))
    model.poll()
    #expect(model.noticeOptions == headsUp)
}

@MainActor
@Test func onlyClaudeCodeOffersNoticeOptions() {
    #expect(noticeShown(FakeCore(), kind: "codex").noticeOptions.isEmpty)
}

@MainActor
@Test func noticeOptionsHideWhileBlockedOrScrolledUp() {
    let core = FakeCore()
    let model = noticeShown(core)
    model.blocked = .optionsOnly
    #expect(model.noticeOptions.isEmpty)
    model.blocked = nil
    #expect(model.noticeOptions == rating)
    core.state.withLock { $0.status = .blocked }
    model.poll()
    #expect(model.blocked == nil && model.noticeOptions.isEmpty)
    let scrolled = noticeShown(FakeCore(), jumpBanner: true)
    #expect(scrolled.jumpBanner && scrolled.noticeOptions.isEmpty)
}

@MainActor
@Test func answeringTypesTheDraftInsteadOfPrompting() async {
    let core = FakeCore()
    let model = agentModel(core)
    await attach(model, core, "a.png")
    model.blocked = .keysAndText
    #expect(model.answering)
    #expect(!model.canSendPrompt)
    model.draft = "  use the staging cluster \n"
    #expect(model.canSendPrompt)
    await model.sendPrompt()
    #expect(core.snapshot.typed == ["use the staging cluster"])
    #expect(core.snapshot.prompts.isEmpty)
    #expect(model.draft.isEmpty)
    #expect(model.attachments.map(\.name) == ["a.png"])

    model.draft = "again"
    core.set(error: .AgentBlocked)
    await model.sendPrompt()
    #expect(model.draft == "again")
    #expect(model.promptError?.contains("Approvals") == true)

    core.set(error: nil)
    model.blocked = nil
    await model.sendPrompt()
    #expect(core.snapshot.typed == ["use the staging cluster", "again"])
    #expect(core.snapshot.prompts == ["/Users/me/Library/Caches/dev.rbstp.collied/attachments/a.png/a.png again"])
}

@MainActor
@Test func codexQuestionCanOpenAndSendWhileApprovalHintIsPresent() async {
    let core = FakeCore()
    core.state.withLock {
        $0.kind = "codex"
        $0.status = .blocked
        $0.output = TerminalSnapshot(
            terminalId: "term_1", source: .recent,
            ansi: "Queued follow-up inputs\n? 1 question\n⇧←\u{1B}[0m to answer\n⚠ 1 warning · f2 to view", truncated: false
        )
    }
    let model = agentModel(core)
    model.poll()
    model.blocked = .terminal
    #expect(model.codexQuestionQueued && !model.answering && !model.canSendPrompt)
    #expect(model.keyStrip[5] == .shiftLeft)
    #expect(model.accepts(.shiftLeft) && !model.accepts(.enter))
    model.typed("other input")
    core.set(error: .AgentBlocked)
    await model.sendPrompt()
    #expect(model.promptError?.contains("⇧←") == true)
    core.set()
    model.typed("")
    await model.tap(.shiftLeft)?.value
    #expect(core.snapshot.keys == [[.shiftLeft]])
    core.state.withLock {
        $0.output = TerminalSnapshot(
            terminalId: "term_1", source: .recent,
            ansi: "Which database?\n› 1. SQLite\n  2. Redis\n  3. Other\nenter\u{1B}[0m submit   ⇧→ main prompt", truncated: false
        )
        $0.outputRevision += 1
    }
    model.poll()
    #expect(model.answering && !model.codexQuestionQueued)
    #expect(model.accepts(.up) && model.accepts(.down) && model.accepts(.enter))
    #expect(!model.accepts(.left) && !model.accepts(.shiftLeft))
    await model.tap(.down)?.value
    await model.tap(.enter)?.value
    #expect(core.snapshot.keys == [[.shiftLeft], [.down], [.enter]])
    model.typed("DuckDB")
    await model.sendPrompt()
    #expect(core.snapshot.typed == ["DuckDB"])
    #expect(core.snapshot.prompts == ["other input"])
}

@MainActor
@Test func claudeKeepsTabInTheKeyStrip() {
    let core = FakeCore()
    core.state.withLock { $0.kind = "claude" }
    let model = agentModel(core)
    model.poll()
    #expect(model.keyStrip[5] == .tab)
    #expect(!model.accepts(.shiftLeft))
}

private final class FakeUnlocker: TerminalUnlocker {
    let signs: Bool
    let passcodeSet: Bool
    let reasons = Mutex<[String]>([])

    init(signs: Bool, passcodeSet: Bool = true) {
        self.signs = signs
        self.passcodeSet = passcodeSet
    }

    func sign(_ message: Data, reason: String) async -> Data? {
        reasons.withLock { $0.append(reason) }
        return signs ? Data("signed \(String(decoding: message, as: UTF8.self))".utf8) : nil
    }
}

private let shellPane = TerminalSummary(
    terminalId: "term_1", workspaceId: "w1", workspaceLabel: "api", label: nil, cwd: "/Users/me/api", locked: true
)

@MainActor
private func terminalModel(_ core: FakeCore, _ unlocker: FakeUnlocker) -> AgentModel {
    AgentModel(
        core: core, route: AgentRoute(machineId: "m1", terminalId: "term_1"), prefsFile: nil,
        unlocker: unlocker, machineLabel: "Mac Studio"
    )
}

@MainActor
@Test func anExitedAgentTurnsIntoALockedShellAndBack() async {
    let core = FakeCore()
    core.state.withLock { $0.kind = "claude" }
    let unlocker = FakeUnlocker(signs: true)
    let model = terminalModel(core, unlocker)
    model.poll()
    #expect(model.mode == .agent)
    #expect(model.paneNotice == nil)

    core.state.withLock {
        $0.kind = nil
        $0.shell = shellPane
    }
    model.poll()
    #expect(model.mode == .terminal)
    #expect(model.terminalLocked)
    #expect(model.paneNotice == "The agent exited. This pane is now a shell.")
    #expect(model.canUnlock)
    #expect(!model.answering)
    #expect(core.snapshot.challenges == 0, "Face ID is never raised on its own")

    #expect(await model.unlock())
    #expect(unlocker.reasons.withLock { $0 } == ["Open a terminal in api on Mac Studio"])
    #expect(core.snapshot.grants == [Data("signed challenge term_1".utf8)])
    model.poll()
    #expect(!model.terminalLocked)
    #expect(model.paneNotice == nil)
    while core.snapshot.shellWatches.isEmpty {
        try? await Task.sleep(for: .milliseconds(5))
    }
    #expect(core.snapshot.shellWatches == ["term_1"])

    core.state.withLock { $0.kind = "claude" }
    model.poll()
    #expect(model.mode == .agent)
    while core.snapshot.watches.isEmpty {
        try? await Task.sleep(for: .milliseconds(5))
    }
    #expect(core.snapshot.watches == ["term_1"], "back to the agent's watch")
}

@MainActor
@Test func aLockedSendUnlocksFirstAndACancelledFaceIDSendsNothing() async {
    let core = FakeCore()
    core.state.withLock { $0.shell = shellPane }
    let cancelled = terminalModel(core, FakeUnlocker(signs: false))
    cancelled.poll()
    cancelled.draft = "git pull"
    #expect(cancelled.canSendPrompt)
    await cancelled.sendPrompt()
    cancelled.tap(.ctrlC)
    try? await Task.sleep(for: .milliseconds(50))
    #expect(core.snapshot.challenges == 2)
    #expect(core.snapshot.grants.isEmpty)
    #expect(core.snapshot.commands.isEmpty && core.snapshot.terminalKeys.isEmpty)
    #expect(cancelled.draft == "git pull")

    let unlocker = FakeUnlocker(signs: true)
    let model = terminalModel(core, unlocker)
    model.poll()
    model.draft = "git pull --force"
    await model.sendPrompt()
    #expect(unlocker.reasons.withLock { $0.count } == 1)
    #expect(core.snapshot.grants.count == 1)
    #expect(core.snapshot.commands == ["git pull --force"])
    #expect(model.draft.isEmpty)
    #expect(core.snapshot.prompts.isEmpty, "a shell is never prompted")

    await model.tap(.ctrlC)?.value
    #expect(core.snapshot.terminalKeys == [[.ctrlC]])
    #expect(core.snapshot.keys.isEmpty)
    #expect(unlocker.reasons.withLock { $0.count } == 1, "one Face ID covers the grant")

    core.set(error: .TerminalLocked)
    model.draft = "ls"
    await model.sendPrompt()
    #expect(model.terminalLocked)
    #expect(model.promptError == "The terminal locked. Unlock it again to continue.")
}

@MainActor
@Test func aShellTakesOneCommandAtATime() async {
    let core = FakeCore()
    core.state.withLock { $0.shell = shellPane }
    let unlocker = FakeUnlocker(signs: true)
    let model = terminalModel(core, unlocker)
    model.poll()
    model.draft = "cd api\ngit pull"
    await model.sendPrompt()
    #expect(model.promptError == "One command at a time")
    #expect(core.snapshot.challenges == 0)
    #expect(core.snapshot.commands.isEmpty)
    #expect(AgentKey.terminalStrip == [.esc, .tab, .ctrlC, .left, .up, .down, .right, .enter])
}

@MainActor
@Test func theScreenSaysWhyNoTerminalOpens() async {
    let core = FakeCore()
    core.state.withLock { $0.kind = "claude" }
    let model = terminalModel(core, FakeUnlocker(signs: true))
    model.poll()
    core.state.withLock {
        $0.kind = nil
        $0.terminalsEnabled = false
    }
    model.poll()
    #expect(model.mode == .gone)
    #expect(
        model.paneNotice
            == "The agent exited. Terminals are off on this machine: set [terminals] enabled = true in collied.toml there and restart collied."
    )

    core.state.withLock { $0.terminalsEnabled = true }
    let opened = terminalModel(core, FakeUnlocker(signs: true))
    opened.poll()
    #expect(opened.mode == nil, "nothing to show for a machine that never answered")

    core.state.withLock {
        $0.shell = shellPane
        $0.terminalError = .TerminalKeyMissing
    }
    let unpaired = terminalModel(core, FakeUnlocker(signs: true))
    unpaired.poll()
    #expect(unpaired.paneNotice == "This pane is a shell.")
    #expect(!(await unpaired.unlock()))
    #expect(unpaired.paneNotice == "Pair this phone again to use terminals on this machine.")
    #expect(unpaired.notice == nil, "the card says it once")
    #expect(!unpaired.canUnlock)

    let noPasscode = terminalModel(core, FakeUnlocker(signs: true, passcodeSet: false))
    noPasscode.poll()
    await noPasscode.unlock()
    #expect(noPasscode.paneNotice == "Set a passcode on this phone, then pair it again to use terminals on this machine.")
}

@MainActor
@Test func inputForTheAgentNeverReachesTheShellItLeaves() async {
    let core = FakeCore()
    core.state.withLock { $0.kind = "claude" }
    let unlocker = FakeUnlocker(signs: true)
    let model = terminalModel(core, unlocker)
    model.poll()
    model.draft = "rm the old logs and rebuild"
    core.set(hold: true)
    let draining = model.tap(.down)
    await core.waitHeld(1)
    model.tap(.enter)

    core.state.withLock {
        $0.kind = nil
        $0.shell = shellPane
        $0.shellLocked = false
    }
    model.poll()
    #expect(model.mode == .terminal)
    #expect(model.draft.isEmpty)
    #expect(!model.canSendPrompt)
    core.release()
    await draining?.value
    #expect(core.snapshot.keys == [[.down]], "the key queued for the agent is dropped")
    #expect(core.snapshot.terminalKeys.isEmpty && core.snapshot.commands.isEmpty)
    #expect(unlocker.reasons.withLock { $0.isEmpty })

    model.draft = "ls"
    core.state.withLock {
        $0.kind = "claude"
        $0.shell = nil
    }
    model.poll()
    #expect(model.mode == .agent)
    #expect(model.draft.isEmpty, "nor a command the agent")
}

@MainActor
@Test func anAgentBackInALockedShellIsWatchedAgain() async {
    let core = FakeCore()
    core.state.withLock { $0.kind = "claude" }
    let model = terminalModel(core, FakeUnlocker(signs: true))
    model.poll()
    core.state.withLock {
        $0.kind = nil
        $0.shell = shellPane
    }
    model.poll()
    #expect(model.mode == .terminal && model.terminalLocked)
    await model.refresh()
    #expect(core.snapshot.reads.isEmpty, "a shell is not read as an agent")

    core.state.withLock { $0.kind = "claude" }
    model.poll()
    while core.snapshot.watches.isEmpty {
        try? await Task.sleep(for: .milliseconds(5))
    }
    #expect(core.snapshot.watches == ["term_1"])
    #expect(core.snapshot.shellWatches.isEmpty)
}

@Test func terminalsAreListedOnlyFromAMachineThatEnablesThem() {
    let machine = Machine(id: "m1", label: "Mac", host: "mac.ts.net", port: 8457, nodeId: "n1", kind: .mac, key: "")
    let named = TerminalSummary(terminalId: "t2", workspaceId: "w1", workspaceLabel: "api", label: " logs ", cwd: "/var/log", locked: true)
    func entry(enabled: Bool) -> MachineFlockEntry {
        MachineFlockEntry(
            machine: machine,
            flock: MachineFlock(
                machine: machine, link: .connected, lastError: nil, details: nil, workspaces: [], agents: [],
                approvalsCount: 0, terminals: [shellPane, named], terminalsEnabled: enabled
            )
        )
    }
    #expect(entry(enabled: false).terminals.isEmpty)
    #expect(entry(enabled: true).terminals.map(\.displayTitle) == ["api", "logs"])
    let bare = TerminalSummary(terminalId: "t3", workspaceId: "w1", workspaceLabel: nil, label: nil, cwd: nil, locked: true)
    #expect(bare.displayTitle == "Terminal")
}

@MainActor
private func slashReady(_ core: FakeCore) async -> AgentModel {
    let model = openedAgent(core, macDraft: "")
    await model.loadMacDraft()
    return model
}

@MainActor
@Test func typingASlashCommandMirrorsOnlyItsToken() async {
    let core = FakeCore()
    let model = await slashReady(core)
    for text in ["/", "/s", "/sk"] { model.typed(text) }
    await model.flushMirror()
    #expect(core.snapshot.slashes == ["/sk"])
    #expect(core.snapshot.slashExpected == [""])
    #expect(model.macDraft == "/sk" && model.commandShown)

    model.typed("/sk ")
    model.typed("/sk some args")
    await model.flushMirror()
    #expect(core.snapshot.slashes == ["/sk"], "arguments stay on the phone")

    model.typed("plain text")
    await model.flushMirror()
    model.typed("plain text, longer")
    await model.flushMirror()
    #expect(core.snapshot.slashes == ["/sk", ""], "cleared once")
    #expect(core.snapshot.slashExpected == ["", "/sk"])
    #expect(model.macDraft == "" && !model.commandShown)
    #expect(core.snapshot.prompts.isEmpty && core.snapshot.keys.isEmpty)

    model.typed("/é")
    await model.flushMirror()
    #expect(core.snapshot.slashes.count == 2, "no command collied would refuse")
}

@MainActor
@Test func aBurstOfTypingMakesFewMirrorCalls() async {
    let core = FakeCore()
    let model = await slashReady(core)
    model.typed("/")
    model.typed("/s")
    for _ in 0..<100 where core.snapshot.slashes.isEmpty {
        try? await Task.sleep(for: .milliseconds(10))
    }
    #expect(core.snapshot.slashes == ["/s"], "debounced")

    core.set(hold: true)
    model.typed("/sk")
    let first = Task { await model.flushMirror() }
    await core.waitHeld(1)
    model.typed("/ski")
    model.typed("/skil")
    core.release()
    await first.value
    await model.flushMirror()
    #expect(core.snapshot.slashes == ["/s", "/sk", "/skil"])
    #expect(core.snapshot.slashExpected == ["", "/s", "/sk"])
}

@MainActor
@Test func onlyKeyboardTypingMirrors() async {
    let core = FakeCore()
    let model = await slashReady(core)
    model.draft = "/s"
    model.paste("k")
    await model.flushMirror()
    await model.tap(.down)?.value
    #expect(core.snapshot.slashes.isEmpty)
    #expect(core.snapshot.keys == [[.down]])

    let codex = openedAgent(core, kind: "codex", macDraft: "")
    await codex.loadMacDraft()
    codex.typed("/s")
    await codex.flushMirror()

    let blocked = await slashReady(core)
    blocked.blocked = .keysAndText
    blocked.typed("/s")
    await blocked.flushMirror()

    let unknown = openedAgent(core, macDraft: nil)
    await unknown.loadMacDraft()
    unknown.typed("/s")
    await unknown.flushMirror()
    #expect(core.snapshot.slashes.isEmpty)

    core.state.withLock {
        $0.output = TerminalSnapshot(terminalId: "term_1", source: .recent, ansi: "Jump to bottom", truncated: false, jumpBanner: true)
    }
    let scrolled = await slashReady(core)
    scrolled.typed("/s")
    await scrolled.flushMirror()
    #expect(scrolled.jumpBanner && core.snapshot.slashes.isEmpty)
}

@MainActor
@Test func aPickedCommandReplacesThePhoneCommand() async {
    let core = FakeCore()
    let model = await slashReady(core)
    model.typed("/sk")
    await model.flushMirror()
    core.state.withLock { $0.macDraft = "/skills" }
    await model.tap(.tab)?.value
    #expect(core.snapshot.keys == [[.tab]])
    #expect(model.draft == "/skills ")
    #expect(model.macDraft == "/skills")

    model.typed("/skills extra")
    await model.flushMirror()
    #expect(core.snapshot.slashes == ["/sk"])
    await model.sendPrompt()
    #expect(core.snapshot.prompts == ["/skills extra"])
    #expect(core.snapshot.expectedDrafts == ["/skills"])
    #expect(model.macDraft == "" && model.draft.isEmpty)

    model.typed("/mod opus")
    await model.flushMirror()
    core.state.withLock { $0.macDraft = "/model  [model]" }
    await model.tap(.tab)?.value
    #expect(model.draft == "/model opus", "the hint Claude Code draws is not taken")
    #expect(model.macDraft == "/model  [model]")
}

@MainActor
@Test func enterNeverRunsAMirroredCommand() async {
    let core = FakeCore()
    let model = await slashReady(core)
    model.typed("/s")
    let enter = model.tap(.enter)
    #expect(enter != nil, "the box is still empty")
    await enter?.value
    #expect(core.snapshot.slashes == ["/s"])
    #expect(core.snapshot.keys.isEmpty, "Enter queued before the mirror is dropped")
    #expect(model.commandShown)
    #expect(model.tap(.enter) == nil && model.tap(.ctrlEnter) == nil)
    await model.tap(.down)?.value
    #expect(core.snapshot.keys == [[.down]])

    model.blocked = .keys
    #expect(!model.commandShown, "Enter answers the question that covers the box")
    await model.tap(.enter)?.value
    #expect(core.snapshot.keys == [[.down], [.enter]])
}

@MainActor
@Test func arrowsNeedTabBeforeSend() async {
    let core = FakeCore()
    let model = await slashReady(core)
    model.typed("/s")
    await model.flushMirror()
    core.state.withLock { $0.macDraft = "/s" }
    await model.tap(.down)?.value
    #expect(model.draft == "/s")
    await model.sendPrompt()
    #expect(core.snapshot.prompts.isEmpty, "the highlighted command is not what the box holds")
    #expect(model.promptError != nil)

    model.typed("/st")
    await model.flushMirror()
    await model.sendPrompt()
    #expect(core.snapshot.prompts == ["/st"], "a new token resets the menu")

    model.typed("/s")
    await model.flushMirror()
    core.state.withLock { $0.macDraft = "/s" }
    await model.tap(.down)?.value
    core.state.withLock { $0.macDraft = "/status" }
    await model.tap(.tab)?.value
    #expect(model.draft == "/status ")
    await model.sendPrompt()
    #expect(core.snapshot.prompts == ["/st", "/status"])
    #expect(core.snapshot.expectedDrafts == ["/st", "/status"])
}

@MainActor
@Test func aSendWaitsForTheMirrorInFlight() async {
    let core = FakeCore()
    let model = await slashReady(core)
    core.set(hold: true)
    model.typed("/skills")
    let mirror = Task { await model.flushMirror() }
    await core.waitHeld(1)
    let send = Task { await model.sendPrompt() }
    try? await Task.sleep(for: .milliseconds(20))
    #expect(core.snapshot.prompts.isEmpty)
    core.release()
    await mirror.value
    await send.value
    #expect(core.snapshot.prompts == ["/skills"])
    #expect(core.snapshot.expectedDrafts == ["/skills"])
}

@MainActor
@Test func textTypedOnTheMacPausesTheMirror() async {
    let core = FakeCore()
    let model = await slashReady(core)
    core.state.withLock { $0.slashErrors = [.DraftChanged(current: "/s")] }
    model.typed("/s")
    await model.flushMirror()
    model.typed("/sk")
    await model.flushMirror()
    #expect(core.snapshot.slashes == ["/s", "/sk"], "a paste whose reply was lost is its own")
    #expect(core.snapshot.slashExpected == ["", "/s"])
    #expect(model.promptError == nil)

    core.state.withLock { $0.slashErrors = [.DraftChanged(current: "/x")] }
    model.typed("/ski")
    await model.flushMirror()
    model.typed("/skil")
    await model.flushMirror()
    #expect(core.snapshot.slashes == ["/s", "/sk", "/ski"], "a command typed on the Mac is not replaced")
    #expect(model.macDraft == "/x" && model.promptError != nil)

    await model.sendPrompt()
    #expect(core.snapshot.expectedDrafts == ["/x"], "shown before the send replaces it")
    core.state.withLock { $0.slashErrors = [.DraftChanged(current: "typed on the Mac")] }
    model.typed("/c")
    await model.flushMirror()
    model.typed("/co")
    await model.flushMirror()
    #expect(core.snapshot.slashes == ["/s", "/sk", "/ski", "/c"], "resumes after a send")
    #expect(model.macDraft == "typed on the Mac" && !model.commandShown)
}

@MainActor
@Test func onlyACompletionOfThePhoneCommandIsTaken() async {
    let core = FakeCore()
    let model = openedAgent(core, macDraft: "/review delete the old branch")
    model.draft = "/review foo"
    await model.loadMacDraft()
    #expect(model.macDraft == nil, "text the phone never showed stays unknown")

    core.set(error: .DraftChanged(current: "/deploy prod"))
    await model.sendPrompt()
    #expect(model.draft == "/review foo" && model.macDraft == "/deploy prod")
    core.set(error: .DraftChanged(current: "/reviewer  [pr]"))
    await model.sendPrompt()
    #expect(model.draft == "/reviewer foo", "a Tab completion missed after the keys")
}

@MainActor
@Test func aReopenedScreenKnowsItsMirroredCommand() async {
    let core = FakeCore()
    let model = openedAgent(core, macDraft: "/s")
    model.draft = "/s keep this"
    await model.loadMacDraft()
    #expect(model.draft == "/s keep this" && model.macDraft == "/s")
    await model.sendPrompt()
    #expect(core.snapshot.expectedDrafts == ["/s"])
}
