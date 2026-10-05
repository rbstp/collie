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

private func entry(_ machineId: String, link: LinkPhase = .connected, _ agents: [(String, AgentState)]) -> MachineFlockEntry {
    let machine = Machine(id: machineId, label: "Mac", host: "mac.ts.net", port: 8457, nodeId: "n1", kind: .mac, key: "")
    let summaries = agents.map {
        AgentSummary(
            terminalId: $0.0, workspaceId: "w1", kind: "claude", name: nil, title: nil,
            status: $0.1, statusSinceMs: 0, cwd: nil, lastLine: nil
        )
    }
    let flock = MachineFlock(machine: machine, link: link, lastError: nil, details: nil, workspaces: [], agents: summaries, approvalsCount: 0)
    return MachineFlockEntry(machine: machine, flock: flock)
}

private func route(_ terminalId: String, on machineId: String = "m1") -> AgentRoute {
    AgentRoute(machineId: machineId, terminalId: terminalId)
}

@MainActor
private func running(_ model: PreviewModel, _ core: FakeCore) async -> Task<Void, Never> {
    let run = Task { await model.run(core: core) }
    while !model.running {
        await Task.yield()
    }
    return run
}

@MainActor
@Test func onlyVisibleCardsOfConnectedMachinesAreRead() async {
    let core = FakeCore()
    let clock = FakeClock()
    let model = PreviewModel { clock.now }
    let run = await running(model, core)
    model.update([entry("m1", [("t1", .working), ("t2", .idle), ("t3", .done)]), entry("m2", link: .unavailable, [("t4", .idle)])])
    model.appeared(route("t1"))
    model.appeared(route("t2"))
    model.appeared(route("t4", on: "m2"))
    await model.tick()?.value
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
@Test func eachVisibleCardIsReadEveryFiveSeconds() async {
    let core = FakeCore()
    let clock = FakeClock()
    let model = PreviewModel { clock.now }
    let run = await running(model, core)
    model.update([entry("m1", [("t1", .working)])])
    model.appeared(route("t1"))
    for _ in 0..<60 {
        await model.tick()?.value
        clock.advance(.seconds(1))
    }
    #expect(core.snapshot.reads.count == 12)
    run.cancel()
    await run.value
}

@MainActor
@Test func aStatusChangeReadsAtOnce() async {
    let core = FakeCore()
    let clock = FakeClock()
    let model = PreviewModel { clock.now }
    let run = await running(model, core)
    model.update([entry("m1", [("t1", .working), ("t2", .working)])])
    model.appeared(route("t1"))
    model.appeared(route("t2"))
    await model.tick()?.value
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
@Test func nothingIsReadOnceTheGridStops() async {
    let core = FakeCore()
    let clock = FakeClock()
    let model = PreviewModel { clock.now }
    model.update([entry("m1", [("t1", .working)])])
    model.appeared(route("t1"))
    #expect(model.tick() == nil)

    let run = await running(model, core)
    while model.screens.isEmpty {
        await Task.yield()
    }
    #expect(core.snapshot.reads.count == 1)
    run.cancel()
    await run.value
    #expect(!model.running)

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
    let run = await running(model, core)
    model.update([entry("m1", [("t1", .working), ("t2", .working)])])
    model.appeared(route("t1"))
    model.appeared(route("t2"))
    await model.tick()?.value
    #expect(core.snapshot.reads == ["t2"])
    #expect(model.screens[route("t1")] == "watched")
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
