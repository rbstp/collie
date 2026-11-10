import CollieCore
import Testing

@testable import Collie

private func orderedMachine(_ id: String) -> Machine {
    Machine(id: id, label: id, host: "\(id).example.ts.net", port: 8457, nodeId: "n\(id)", kind: .mac, key: "")
}

@Test func newlyPairedMachinesAppendAndRemovalKeepsTheRemainingOrder() {
    let order = MachineOrder(ids: ["linux", "removed", "mac"])
    let machines = [orderedMachine("new"), orderedMachine("mac"), orderedMachine("linux")]
    #expect(order.sorted(machines).map(\.id) == ["linux", "mac", "new"])
    #expect(MachineOrder().sorted(machines) == machines)
}

@MainActor
@Test func agentsUseTheSavedMachineOrderAcrossRefreshAndReconnect() async {
    let core = FakeCore()
    let mac = orderedMachine("mac")
    let linux = orderedMachine("linux")
    let agents = [
        AgentSummary(terminalId: "done", workspaceId: "w1", kind: "codex", name: nil, title: nil,
                     status: .done, statusSinceMs: 0, cwd: nil, lastLine: nil),
        AgentSummary(terminalId: "blocked", workspaceId: "w1", kind: "claude", name: nil, title: nil,
                     status: .blocked, statusSinceMs: 0, cwd: nil, lastLine: nil),
    ]
    core.state.withLock {
        $0.machines = [mac, linux]
        $0.cachedFlock = MachineFlock(machine: mac, link: .offline, lastError: nil, details: nil,
                                     workspaces: [], agents: agents, approvalsCount: 0)
    }
    let model = FlockModel()
    let ordered = MachineOrder(ids: ["linux", "mac"]).sorted(core.machines())
    await model.refresh(core: core, machines: ordered, snapshot: false)
    #expect(model.entries.map(\.id) == ["linux", "mac"])
    #expect(model.entries.allSatisfy { $0.flock?.link == .offline })
    #expect(model.entries.allSatisfy { $0.agents.map(\.terminalId) == ["blocked", "done"] })
    #expect(FlockOrder.neighbor(of: route("done", on: "linux"), offset: 1, in: FlockOrder.routes(in: model.entries), among: model.entries) == route("blocked", on: "mac"))

    core.state.withLock { $0.cachedFlock?.link = .connected }
    await model.refresh(core: core, machines: ordered, snapshot: false)
    #expect(model.entries.map(\.id) == ["linux", "mac"])
    #expect(model.entries.allSatisfy { $0.flock?.link == .connected })
    #expect(core.snapshot.flocks == 0)

    await model.refresh(core: core, machines: [mac, linux], snapshot: false)
    #expect(model.entries.map(\.id) == ["mac", "linux"])
    #expect(model.entries.allSatisfy { $0.agents.map(\.terminalId) == ["blocked", "done"] })
}
