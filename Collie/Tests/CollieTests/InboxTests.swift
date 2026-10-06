import CollieCore
import Foundation
import SwiftUI
import Testing

@testable import Collie

private let now = Date(timeIntervalSince1970: 2_000_000_000)
private let nowMs = UInt64(2_000_000_000_000)

private func agent(_ id: String, _ status: AgentState, activity: UInt64?, since: UInt64 = 0) -> AgentSummary {
    AgentSummary(
        terminalId: id, workspaceId: "w1", kind: "claude", name: nil, title: nil,
        status: status, statusSinceMs: since, cwd: nil, lastLine: nil, lastActivityMs: activity
    )
}

private func entry(_ machineId: String, _ agents: [AgentSummary]) -> MachineFlockEntry {
    let machine = Machine(id: machineId, label: "Mac \(machineId)", host: "mac.ts.net", port: 8457, nodeId: machineId, kind: .mac, key: "")
    let workspaces = [WorkspaceSummary(workspaceId: "w1", label: "collie", number: 1, status: .idle, cwd: nil)]
    let flock = MachineFlock(
        machine: machine, link: .connected, lastError: nil, details: nil, workspaces: workspaces, agents: agents, approvalsCount: 0
    )
    return MachineFlockEntry(machine: machine, flock: flock)
}

@Test func inboxSectionsFollowStatusAndLastActivity() {
    let hour: UInt64 = 3_600_000
    #expect(InboxSection.of(agent("a", .working, activity: nil), now: now) == .working)
    #expect(InboxSection.of(agent("a", .blocked, activity: nil), now: now) == .working)
    #expect(InboxSection.of(agent("a", .done, activity: nil), now: now) == .done)
    #expect(InboxSection.of(agent("a", .idle, activity: nowMs - 23 * hour), now: now) == .done)
    #expect(InboxSection.of(agent("a", .unknown, activity: nowMs - hour), now: now) == .done)
    #expect(InboxSection.of(agent("a", .idle, activity: nowMs - 25 * hour), now: now) == .archived)
    #expect(InboxSection.of(agent("a", .unknown, activity: nil), now: now) == .archived)
}

@Test func inboxPutsBlockedFirstThenTheMostRecentAcrossMachines() {
    let entries = [
        entry("m1", [agent("old", .working, activity: 10), agent("blocked", .blocked, activity: 1), agent("idle", .idle, activity: nil)]),
        entry("m2", [agent("new", .working, activity: 30), agent("since", .done, activity: nil, since: 50), agent("done", .done, activity: 40)]),
    ]
    let items = InboxItem.items(in: entries)
    #expect(items.first { $0.route.terminalId == "new" }?.machine == "Mac m2")
    #expect(items.first { $0.route.terminalId == "old" }?.workspace == "collie")
    let groups = InboxSection.grouped(items, now: now)
    #expect(groups[.working]?.map(\.route.terminalId) == ["blocked", "new", "old"])
    #expect(groups[.done]?.map(\.route.terminalId) == ["since", "done"])
    #expect(groups[.archived]?.map(\.route) == [AgentRoute(machineId: "m1", terminalId: "idle")])
}

@MainActor
@Test func contextRingTonesFollowTheAmountLeft() {
    #expect(ContextRing.tone(100) == ContextRing.tone(51))
    #expect(ContextRing.tone(50) == ContextRing.tone(20))
    #expect(ContextRing.tone(19) == ContextRing.tone(0))
    #expect(ContextRing.tone(51) != ContextRing.tone(50))
    #expect(ContextRing.tone(20) != ContextRing.tone(19))
}
