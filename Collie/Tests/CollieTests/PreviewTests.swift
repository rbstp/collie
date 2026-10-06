import CollieCore
import Foundation
import Testing

@testable import Collie

@MainActor
private final class FakeClock {
    var now = ContinuousClock.Instant.now

    func advance(_ duration: Duration) {
        now = now.advanced(by: duration)
    }
}

private func entry(_ machineId: String, link: LinkPhase = .connected, since: UInt64 = 0, _ agents: [(String, AgentState)]) -> MachineFlockEntry {
    let machine = Machine(id: machineId, label: "Mac", host: "mac.ts.net", port: 8457, nodeId: "n1", kind: .mac, key: "")
    let summaries = agents.map {
        AgentSummary(
            terminalId: $0.0, workspaceId: "w1", kind: "claude", name: nil, title: nil,
            status: $0.1, statusSinceMs: since, cwd: nil, lastLine: nil
        )
    }
    let flock = MachineFlock(machine: machine, link: link, lastError: nil, details: nil, workspaces: [], agents: summaries, approvalsCount: 0)
    return MachineFlockEntry(machine: machine, flock: flock)
}

private func route(_ terminalId: String, on machineId: String = "m1") -> AgentRoute {
    AgentRoute(machineId: machineId, terminalId: terminalId)
}

@MainActor
private func running(_ model: PreviewModel, _ core: FakeCore, screens: Int) async -> Task<Void, Never> {
    let run = Task { await model.run(core: core) }
    while model.screens.count < screens {
        await Task.yield()
    }
    return run
}

@MainActor
@Test func onlyVisibleCardsOfConnectedMachinesAreRead() async {
    let core = FakeCore()
    let clock = FakeClock()
    let model = PreviewModel { clock.now }
    model.update([entry("m1", [("t1", .working), ("t2", .working), ("t3", .done)]), entry("m2", link: .unavailable, [("t4", .idle)])])
    model.appeared(route("t1"))
    model.appeared(route("t2"))
    model.appeared(route("t4", on: "m2"))
    let run = await running(model, core, screens: 2)
    #expect(core.snapshot.reads.sorted() == ["t1", "t2"])
    #expect(model.screens[route("t1")] != nil && model.screens[route("t3")] == nil)

    model.disappeared(route("t2"))
    clock.advance(.seconds(5))
    await model.tick()?.value
    #expect(core.snapshot.reads.sorted() == ["t1", "t1", "t2"])
    run.cancel()
    await run.value
}

@MainActor
@Test func aCardReadsOnlyTheLinesItCanShow() async {
    let core = FakeCore()
    let model = PreviewModel()
    model.update([entry("m1", [("t1", .working)])])
    model.appeared(route("t1"))
    let run = await running(model, core, screens: 1)
    #expect(core.snapshot.readLines == [60])
    run.cancel()
    await run.value
}

@MainActor
@Test func aWorkingCardIsReadEveryFiveSeconds() async {
    let core = FakeCore()
    let clock = FakeClock()
    let model = PreviewModel { clock.now }
    model.update([entry("m1", [("t1", .working)])])
    model.appeared(route("t1"))
    let run = await running(model, core, screens: 1)
    for _ in 1..<60 {
        clock.advance(.seconds(1))
        await model.tick()?.value
    }
    #expect(core.snapshot.reads.count == 12)
    run.cancel()
    await run.value
}

@MainActor
@Test func aCardThatIsNotWorkingIsReadOnlyWhenItShowsOrItsStatusChanges() async {
    let core = FakeCore()
    let clock = FakeClock()
    let model = PreviewModel { clock.now }
    model.update([entry("m1", [("t1", .idle), ("t2", .done), ("t3", .blocked), ("t4", .unknown)])])
    for id in ["t1", "t2", "t3", "t4"] {
        model.appeared(route(id))
    }
    let run = await running(model, core, screens: 4)
    for _ in 0..<60 {
        clock.advance(.seconds(1))
        await model.tick()?.value
    }
    #expect(core.snapshot.reads.filter { $0 != "t4" }.sorted() == ["t1", "t2", "t3"])
    #expect(core.snapshot.reads.filter { $0 == "t4" }.count == 13)

    model.disappeared(route("t1"))
    model.appeared(route("t1"))
    #expect(model.tick() == nil)
    await model.update([entry("m1", [("t1", .working), ("t2", .done), ("t3", .blocked), ("t4", .unknown)])])?.value
    #expect(core.snapshot.reads.last == "t1" && core.snapshot.reads.filter { $0 == "t1" }.count == 2)
    run.cancel()
    await run.value
}

@MainActor
@Test func aStatusChangeReadsAtOnce() async {
    let core = FakeCore()
    let clock = FakeClock()
    let model = PreviewModel { clock.now }
    model.update([entry("m1", [("t1", .working), ("t2", .working)])])
    model.appeared(route("t1"))
    model.appeared(route("t2"))
    let run = await running(model, core, screens: 2)
    #expect(core.snapshot.reads.count == 2)

    clock.advance(.seconds(1))
    await model.update([entry("m1", [("t1", .working), ("t2", .working)])])?.value
    #expect(core.snapshot.reads.count == 2)
    await model.update([entry("m1", [("t1", .blocked), ("t2", .working)])])?.value
    #expect(core.snapshot.reads.count == 3 && core.snapshot.reads.last == "t1")
    #expect(model.screens[route("t1")] == "read 3")
    run.cancel()
    await run.value
}

@MainActor
@Test func aStatusThatChangedAndCameBackBetweenPollsReadsAtOnce() async {
    let core = FakeCore()
    let clock = FakeClock()
    let model = PreviewModel { clock.now }
    model.update([entry("m1", [("t1", .idle)])])
    model.appeared(route("t1"))
    let run = await running(model, core, screens: 1)

    await model.update([entry("m1", since: 1, [("t1", .idle)])])?.value
    #expect(core.snapshot.reads == ["t1", "t1"])
    run.cancel()
    await run.value
}

@MainActor
@Test func aFailedReadIsTriedAgainAfterTheInterval() async {
    let core = FakeCore()
    let clock = FakeClock()
    let model = PreviewModel { clock.now }
    model.update([entry("m1", [("t1", .idle)])])
    model.appeared(route("t1"))
    let run = await running(model, core, screens: 1)

    core.state.withLock { $0.readError = .MachineNotFound }
    await model.update([entry("m1", [("t1", .done)])])?.value
    #expect(core.snapshot.reads.count == 2 && model.screens[route("t1")] == "read 1")

    core.state.withLock { $0.readError = nil }
    clock.advance(.seconds(4))
    #expect(model.tick() == nil)
    clock.advance(.seconds(1))
    await model.tick()?.value
    #expect(model.screens[route("t1")] == "read 3")
    clock.advance(.seconds(5))
    #expect(model.tick() == nil)
    run.cancel()
    await run.value
}

@MainActor
@Test func nothingIsReadOnceTheGridStops() async {
    let core = FakeCore()
    let clock = FakeClock()
    let model = PreviewModel { clock.now }
    model.update([entry("m1", [("t1", .working)])])
    model.appeared(route("t1"))
    #expect(model.tick() == nil)

    let run = await running(model, core, screens: 1)
    #expect(core.snapshot.reads.count == 1)
    run.cancel()
    await run.value

    clock.advance(.seconds(60))
    #expect(model.tick() == nil)
    #expect(model.update([entry("m1", [("t1", .blocked)])]) == nil)
    #expect(core.snapshot.reads.count == 1)
}

@MainActor
@Test func aWatchedAgentsOutputIsUsedInsteadOfARead() async {
    let core = FakeCore()
    core.state.withLock { $0.output = TerminalSnapshot(terminalId: "t1", source: .recent, ansi: "watched", truncated: false) }
    let clock = FakeClock()
    let model = PreviewModel { clock.now }
    model.update([entry("m1", [("t1", .working), ("t2", .working)])])
    model.appeared(route("t1"))
    model.appeared(route("t2"))
    let run = await running(model, core, screens: 2)
    #expect(core.snapshot.reads == ["t2"])
    #expect(model.screens[route("t1")] == "watched")
    run.cancel()
    await run.value
}

private let output = """
    \u{1b}[38;2;153;153;153m⏺\u{1b}[39m Update(Collie/Sources/Collie/AgentGrid.swift)
      ⎿  \u{1b}[2mUpdated AgentGrid.swift with 12 additions and 3 removals\u{1b}[22m

    ⏺ Done. \u{1b}[1mThe build passes.\u{1b}[22m
    """
private let rule = "\u{1b}[38;2;136;136;136m" + String(repeating: "─", count: 60) + "\u{1b}[39m"
private let labeledRule = "\u{1b}[38;2;136;136;136m" + String(repeating: "─", count: 48) + " ultracode ─\u{1b}[39m"
private let status = """
      \u{1b}[38;2;153;153;153mOpus 5.5 high · ~/GitHub/collie (agent-previews) · 42% used\u{1b}[39m
      \u{1b}[38;2;215;119;87m⏵⏵ auto mode on\u{1b}[39m \u{1b}[2m(shift+tab to cycle)\u{1b}[22m
      ◼ wf_90782106 · implement 2/4 · 3m 12s
    """

private func claude(_ box: String, top: String = rule) -> String {
    "\(output)\n\n\(top)\n\(box)\n\(rule)\n\(status)\n"
}

@Test func aCardEndsAboveClaudeCodesInputBox() {
    for box in [
        "❯\u{a0}\u{1b}[2mTry \"fix lint errors\"\u{1b}[22m",
        "❯ add a test for the crop",
        "❯ first line\n  second line\n\n  \u{1b}[1mfourth\u{1b}[22m",
        "! git status",
    ] {
        #expect(PreviewModel.card(claude(box)) == output, "\(box)")
    }
    #expect(PreviewModel.card(claude("❯ ", top: labeledRule)) == output)
    let crlf = claude("❯ typed").replacing("\n", with: "\r\n")
    #expect(PreviewModel.card(crlf) == output.replacing("\n", with: "\r\n"))
    let typedRule = "❯ one\n  ────────────\n  two"
    #expect(PreviewModel.card(claude(typedRule)) == output, "a typed rule is indented, so it does not close the box")
}

@Test func aCardEndsAboveTheLastInputBox() {
    let stale = "\(rule)\n❯ an old prompt\n\(rule)\n\(output)"
    #expect(PreviewModel.card(claude("❯ ").replacing(output, with: stale)) == stale)
}

@Test func aDialogInPlaceOfTheInputBoxStaysOnTheCard() {
    let bash = """
        \(output)

        \(String(repeating: "─", count: 60))
         Bash command

           cargo test -p collied
           Run the collied tests

         Do you want to proceed?
         ❯ 1. Yes
           2. Yes, and don't ask again for cargo test commands in /Users/me/src/collie
           3. No, and tell Claude what to do differently (esc)

         Esc to cancel · Tab to amend · ctrl+e to explain

        """
    let question = """
        ❯ Use the AskUserQuestion tool to ask me which storage backend the cache should use.
        \(String(repeating: "─", count: 60))
         ☐ Cache Backend

        Which storage backend should the cache use?

        ❯ 1. SQLite
             File-based database, good for single-instance deployments
          2. Redis
             In-memory data store, better for distributed systems
          3. Type something.
        \(String(repeating: "─", count: 60))
          4. Chat about this

        Enter to select · ↑/↓ to navigate · Esc to cancel

        """
    let plan = """
        \(output)

          \(String(repeating: "─", count: 60))
           Ready to code?

           Here is Claude's plan:
          \(String(repeating: "╌", count: 60))
           Create hello.txt containing hi.
          \(String(repeating: "╌", count: 60))
           Claude has written up a plan and is ready to execute. Would you like to proceed?

           ❯ 1. Yes, and use auto mode
             2. Yes, manually approve edits
             3. Tell Claude what to change

        """
    for screen in [bash, question, plan] {
        #expect(PreviewModel.card(screen) == screen)
    }
}

@Test func aScreenWithoutAnInputBoxIsKeptWhole() {
    for screen in [
        "",
        "$ ls\nCargo.toml  Collie  crates\n$ ",
        "\(output)\n\n\(rule)\n❯ a box with no bottom rule",
        "\(rule)\n\n\(rule)\n  status",
        "  rows of a draft\n  taller than the screen\n\(rule)\n\(status)",
        "\u{1b}[1m›\u{1b}[22m Explain this codebase\n\n  ⏎ send   ⇧⏎ newline   ⌃T transcript   ⌃C quit",
    ] {
        #expect(PreviewModel.card(screen) == screen)
    }
}

@MainActor
@Test func aCardOfAClaudeCodeScreenEndsAboveItsInputBox() async {
    let core = FakeCore()
    core.state.withLock { $0.output = TerminalSnapshot(terminalId: "t1", source: .recent, ansi: claude("❯ "), truncated: false) }
    let model = PreviewModel()
    model.update([entry("m1", [("t1", .working)])])
    model.appeared(route("t1"))
    let run = await running(model, core, screens: 1)
    #expect(model.screens[route("t1")] == output)
    run.cancel()
    await run.value
}

@MainActor
@Test func gridChoiceIsOffByDefaultAndRememberedOnThisDevice() throws {
    let dir = FileManager.default.temporaryDirectory.appending(path: UUID().uuidString, directoryHint: .isDirectory)
    try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: dir) }
    let file = dir.appending(path: "prefs.json")

    try Data(#"{"wrapLines":false}"#.utf8).write(to: file)
    #expect(!DevicePrefs.load(from: file).agentsGrid)
    var prefs = DevicePrefs.load(from: file)
    prefs.agentsGrid = true
    prefs.save(to: file)
    #expect(DevicePrefs.load(from: file) == DevicePrefs(wrapLines: false, agentsGrid: true))
}
