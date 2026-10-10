import CollieCore
import Foundation
import SwiftUI
import Testing

@testable import Collie

private let now = Date(timeIntervalSince1970: 2_000_000_000)
private let nowMs = UInt64(2_000_000_000_000)

private func agent(
    _ id: String, _ status: AgentState, activity: UInt64?, since: UInt64 = 0,
    kind: String? = "claude", name: String? = nil, title: String? = nil
) -> AgentSummary {
    AgentSummary(
        terminalId: id, workspaceId: "w1", kind: kind, name: name, title: title,
        status: status, statusSinceMs: since, cwd: nil, lastLine: nil, lastActivityMs: activity
    )
}

private func entry(
    _ machineId: String, _ agents: [AgentSummary], link: LinkPhase = .connected, loaded: Bool = false
) -> MachineFlockEntry {
    let machine = Machine(id: machineId, label: "Mac \(machineId)", host: "mac.ts.net", port: 8457, nodeId: machineId, kind: .mac, key: "")
    let workspaces = [WorkspaceSummary(workspaceId: "w1", label: "collie", number: 1, status: .idle, cwd: nil)]
    let flock = MachineFlock(
        machine: machine, link: link, lastError: nil,
        details: loaded ? MachineDetails(name: machine.label, nodeId: machineId, herdrSession: "session") : nil,
        workspaces: workspaces, agents: agents, approvalsCount: 0
    )
    return MachineFlockEntry(machine: machine, flock: flock)
}

@Test func inboxSearchAndFiltersIncludeCachedMachines() {
    let blocked = agent("blocked", .blocked, activity: 10, name: "Build")
    let finished = agent("done", .done, activity: nowMs - 20, since: nowMs - 30, title: "Review")
    let inactive = agent("old", .idle, activity: nowMs - 25 * 3_600_000, kind: "codex")
    let olderDone = agent("older-done", .done, activity: nowMs - 26 * 3_600_000)
    let items = InboxItem.items(in: [entry("m1", [blocked]), entry("m2", [finished, inactive, olderDone], link: .offline)])
    let seen = SeenAgents()
    #expect(items.first { $0.route.terminalId == "done" }?.stale == true)
    #expect(items.filter { $0.matches("review") }.map(\.route.terminalId) == ["done"])
    #expect(items.filter { $0.matches("build") }.map(\.route.terminalId) == ["blocked"])
    #expect(items.filter { $0.matches("collie") }.count == 4)
    #expect(items.filter { $0.matches("mac m2") }.count == 3)
    #expect(items.filter { $0.matches("CODEX") }.map(\.route.terminalId) == ["old"])
    #expect(Set(items.filter { InboxFilter.needsAttention.includes($0, seen: seen, now: now) }.map(\.route.terminalId))
            == Set(["blocked", "done", "older-done"]))
    #expect(items.filter { InboxFilter.working.includes($0, seen: seen, now: now) }.map(\.route.terminalId) == ["blocked"])
    #expect(items.filter { InboxFilter.inactive.includes($0, seen: seen, now: now) }.map(\.route.terminalId) == ["old", "older-done"])
}

@Test func seenCompletionSurvivesReloadAndNewCompletionNeedsAttention() throws {
    let file = FileManager.default.temporaryDirectory.appending(path: UUID().uuidString)
    defer { try? FileManager.default.removeItem(at: file) }
    let route = AgentRoute(machineId: "m1", terminalId: "done")
    let first = agent("done", .done, activity: 10, since: 5)
    var seen = SeenAgents()
    seen.markOpened(first, route: route, nowMs: 100)
    seen.save(to: file)
    seen = SeenAgents.load(from: file)
    #expect(seen.isSeen(first, route: route))
    #expect(!InboxFilter.needsAttention.includes(InboxItem.items(in: [entry("m1", [first])])[0], seen: seen, now: now))
    let later = agent("done", .done, activity: 20, since: 15)
    #expect(!seen.isSeen(later, route: route))
    seen.markOpened(agent("done", .working, activity: 20, since: 15), route: route, nowMs: 150)
    #expect(!seen.isSeen(later, route: route))
    seen.markOpened(later, route: route, nowMs: 200)
    seen.markUnseen(later, route: route)
    #expect(!seen.isSeen(later, route: route))
    let blocked = agent("done", .blocked, activity: 30, since: 15)
    #expect(InboxFilter.needsAttention.includes(InboxItem.items(in: [entry("m1", [blocked])])[0], seen: seen, now: now))
}

@Test func seenMarkersAreBoundedAndPrunedOnlyFromLoadedMachines() {
    var seen = SeenAgents()
    for index in 0...SeenAgents.limit {
        let route = AgentRoute(machineId: "m1", terminalId: "a\(index)")
        seen.markOpened(agent(route.terminalId, .done, activity: nil), route: route, nowMs: UInt64(index))
    }
    #expect(seen.markers.count == SeenAgents.limit)
    #expect(seen.markers[AgentRoute(machineId: "m1", terminalId: "a0")] == nil)
    seen.prune([entry("m1", [], link: .offline)])
    #expect(seen.markers.count == SeenAgents.limit)
    seen.prune([entry("m1", [agent("a1", .done, activity: nil)], loaded: true)])
    #expect(Array(seen.markers.keys) == [AgentRoute(machineId: "m1", terminalId: "a1")])
    seen.prune([entry("m2", [])])
    #expect(seen.markers.isEmpty)
    seen.markOpened(agent("a", .done, activity: nil), route: AgentRoute(machineId: "m1", terminalId: "a"), nowMs: 1000)
    seen.prune([])
    #expect(seen.markers.isEmpty)
}

@Test func inboxSectionsFollowStatusAndLastActivity() {
    let hour: UInt64 = 3_600_000
    #expect(InboxSection.of(agent("a", .working, activity: nil), now: now) == .working)
    #expect(InboxSection.of(agent("a", .blocked, activity: nil), now: now) == .working)
    #expect(InboxSection.of(agent("a", .done, activity: nil, since: nowMs - hour), now: now) == .done)
    #expect(InboxSection.of(agent("a", .done, activity: nil, since: nowMs - 25 * hour), now: now) == .inactive)
    #expect(InboxSection.of(agent("a", .done, activity: nowMs - 25 * hour), now: now) == .inactive)
    #expect(InboxSection.of(agent("a", .done, activity: nowMs - 24 * hour), now: now) == .inactive)
    #expect(InboxSection.of(agent("a", .blocked, activity: nowMs - 25 * hour), now: now) == .working)
    #expect(InboxSection.of(agent("a", .idle, activity: nowMs - 23 * hour), now: now) == .done)
    #expect(InboxSection.of(agent("a", .unknown, activity: nowMs - hour), now: now) == .done)
    #expect(InboxSection.of(agent("a", .idle, activity: nowMs - 25 * hour), now: now) == .inactive)
    #expect(InboxSection.of(agent("a", .unknown, activity: nil), now: now) == .inactive)
    #expect(InboxSection.of(agent("a", .idle, activity: nil, since: nowMs - hour), now: now) == .done)
}

@Test func inboxPutsBlockedFirstThenTheMostRecentAcrossMachines() {
    let entries = [
        entry("m1", [agent("old", .working, activity: 10), agent("blocked", .blocked, activity: 1), agent("idle", .idle, activity: nil)]),
        entry("m2", [agent("new", .working, activity: 30), agent("since", .done, activity: nil, since: nowMs - 50),
                     agent("done", .done, activity: nowMs - 40)]),
    ]
    let items = InboxItem.items(in: entries)
    #expect(items.first { $0.route.terminalId == "new" }?.machine == "Mac m2")
    #expect(items.first { $0.route.terminalId == "old" }?.workspace == "collie")
    let groups = InboxSection.grouped(items, now: now)
    #expect(groups[.working]?.map(\.route.terminalId) == ["blocked", "new", "old"])
    #expect(groups[.done]?.map(\.route.terminalId) == ["done", "since"])
    #expect(groups[.inactive]?.map(\.route) == [AgentRoute(machineId: "m1", terminalId: "idle")])
}

@MainActor
@Test func contextRingTonesFollowTheAmountLeft() {
    #expect(ContextRing.tone(100) == ContextRing.tone(51))
    #expect(ContextRing.tone(50) == ContextRing.tone(20))
    #expect(ContextRing.tone(19) == ContextRing.tone(0))
    #expect(ContextRing.tone(51) != ContextRing.tone(50))
    #expect(ContextRing.tone(20) != ContextRing.tone(19))
}
