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
    let olderDone = agent("older-done", .done, activity: nowMs - 20, since: nowMs - 26 * 3_600_000)
    let items = InboxItem.items(in: [entry("m1", [blocked]), entry("m2", [finished, inactive, olderDone], link: .offline)])
    #expect(items.first { $0.route.terminalId == "done" }?.stale == true)
    #expect(items.filter { $0.matches("review") }.map(\.route.terminalId) == ["done"])
    #expect(items.filter { $0.matches("build") }.map(\.route.terminalId) == ["blocked"])
    #expect(items.filter { $0.matches("collie") }.count == 4)
    #expect(items.filter { $0.matches("mac m2") }.count == 3)
    #expect(items.filter { $0.matches("CODEX") }.map(\.route.terminalId) == ["old"])
    #expect(items.filter { InboxFilter.needsYou.includes($0, now: now) }.map(\.route.terminalId) == ["blocked"])
    #expect(items.filter { InboxFilter.working.includes($0, now: now) }.isEmpty)
    #expect(items.filter { InboxFilter.done.includes($0, now: now) }.map(\.route.terminalId) == ["done"])
    #expect(items.filter { InboxFilter.inactive.includes($0, now: now) }.map(\.route.terminalId) == ["old", "older-done"])
    #expect(items.filter { InboxFilter.all.includes($0, now: now) }.count == 4)
    #expect(InboxFilter.allCases.compactMap(\.section) == InboxSection.allCases)
}

@Test func seenCompletionSurvivesReloadAndANewCompletionIsUnseen() throws {
    let file = FileManager.default.temporaryDirectory.appending(path: UUID().uuidString)
    defer { try? FileManager.default.removeItem(at: file) }
    let route = AgentRoute(machineId: "m1", terminalId: "done")
    let first = agent("done", .done, activity: 10, since: 5)
    var seen = SeenAgents()
    seen.markOpened(first, route: route, nowMs: 100)
    seen.save(to: file)
    seen = SeenAgents.load(from: file)
    #expect(seen.isSeen(first, route: route))
    let later = agent("done", .done, activity: 20, since: 15)
    #expect(!seen.isSeen(later, route: route))
    seen.markOpened(agent("done", .working, activity: 20, since: 15), route: route, nowMs: 150)
    #expect(!seen.isSeen(later, route: route))
    seen.markOpened(later, route: route, nowMs: 200)
    seen.markUnseen(later, route: route)
    #expect(!seen.isSeen(later, route: route))
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

@Test func snoozeLastsUntilItsWakeTimeAndSurvivesReload() {
    let file = FileManager.default.temporaryDirectory.appending(path: UUID().uuidString)
    defer { try? FileManager.default.removeItem(at: file) }
    let route = AgentRoute(machineId: "m1", terminalId: "a")
    let working = agent("a", .working, activity: nowMs, since: 5)
    let wake = now.addingTimeInterval(3600)
    var snoozed = SnoozedAgents()
    snoozed.snooze(working, route: route, until: wake)
    snoozed.save(to: file)
    snoozed = SnoozedAgents.load(from: file)
    #expect(snoozed.wake(of: working, route: route, now: now) == wake)
    #expect(snoozed.wake(of: working, route: route, now: wake.addingTimeInterval(-1)) == wake)
    #expect(snoozed.wake(of: working, route: route, now: wake) == nil)
    #expect(snoozed.wake(of: working, route: AgentRoute(machineId: "m2", terminalId: "a"), now: now) == nil)
    snoozed.wakeNow(route)
    #expect(snoozed.wake(of: working, route: route, now: now) == nil)
}

@Test func snoozeWakesWhenTheAgentBlocksOrFinishesANewTurn() {
    let route = AgentRoute(machineId: "m1", terminalId: "a")
    let wake = now.addingTimeInterval(3600)
    var snoozed = SnoozedAgents()
    snoozed.snooze(agent("a", .working, activity: nowMs, since: 5), route: route, until: wake)
    #expect(snoozed.wake(of: agent("a", .idle, activity: nowMs, since: 9), route: route, now: now) == wake)
    #expect(snoozed.wake(of: agent("a", .blocked, activity: nowMs, since: 9), route: route, now: now) == nil)
    #expect(snoozed.wake(of: agent("a", .done, activity: nowMs, since: 9), route: route, now: now) == nil)
    snoozed.snooze(agent("a", .done, activity: nowMs, since: 5), route: route, until: wake)
    #expect(snoozed.wake(of: agent("a", .done, activity: nowMs, since: 5), route: route, now: now) == wake)
    #expect(snoozed.wake(of: agent("a", .done, activity: nowMs, since: 9), route: route, now: now) == nil)
    snoozed.snooze(agent("a", .blocked, activity: nowMs, since: 5), route: route, until: wake)
    #expect(snoozed.wake(of: agent("a", .blocked, activity: nowMs, since: 5), route: route, now: now) == nil)
    #expect(snoozed.wake(of: agent("a", .working, activity: nowMs, since: 9), route: route, now: now) == wake)
}

@Test func anEarlyWakeIsFinal() {
    let route = AgentRoute(machineId: "m1", terminalId: "a")
    let wake = now.addingTimeInterval(3600)
    var snoozed = SnoozedAgents()
    for woke in [agent("a", .blocked, activity: nowMs, since: 7), agent("a", .done, activity: nowMs, since: 7)] {
        snoozed.snooze(agent("a", .working, activity: nowMs, since: 5), route: route, until: wake)
        snoozed.endWoken([entry("m1", [agent("a", .working, activity: nowMs, since: 5)], link: .offline)])
        #expect(snoozed.wake(of: agent("a", .working, activity: nowMs, since: 5), route: route, now: now) == wake)
        snoozed.endWoken([entry("m1", [woke], link: .offline)])
        #expect(snoozed.snoozes.isEmpty)
        #expect(snoozed.wake(of: agent("a", .working, activity: nowMs, since: 9), route: route, now: now) == nil)
    }
    snoozed.snooze(agent("a", .working, activity: nowMs, since: 5), route: route, until: wake)
    let first = snoozed.wakeNow(route)
    let second = snoozed.wakeNow(route)
    #expect(first && !second)
}

@Test func snoozesArePrunedWhenGoneWokenOrExpired() {
    let file = FileManager.default.temporaryDirectory.appending(path: UUID().uuidString)
    defer { try? FileManager.default.removeItem(at: file) }
    let wake = now.addingTimeInterval(3600)
    var snoozed = SnoozedAgents()
    for id in ["kept", "closed", "woken", "expired"] {
        snoozed.snooze(agent(id, .working, activity: nil, since: 5), route: AgentRoute(machineId: "m1", terminalId: id),
                       until: id == "expired" ? now : wake)
    }
    snoozed.snooze(agent("a", .working, activity: nil, since: 5), route: AgentRoute(machineId: "m2", terminalId: "a"), until: wake)
    snoozed.prune([entry("m1", [], link: .offline), entry("m2", [])], now: now)
    #expect(Set(snoozed.snoozes.keys.map(\.terminalId)) == Set(["kept", "closed", "woken", "a"]))
    let loaded = entry("m1", [agent("kept", .working, activity: nil, since: 5), agent("woken", .blocked, activity: nil, since: 9)], loaded: true)
    snoozed.prune([loaded, entry("m2", [])], now: now)
    #expect(Set(snoozed.snoozes.keys.map(\.terminalId)) == Set(["kept", "a"]))
    snoozed.prune([loaded], now: now)
    #expect(Array(snoozed.snoozes.keys) == [AgentRoute(machineId: "m1", terminalId: "kept")])
    snoozed.save(to: file)
    SnoozedAgents.forget(machineId: "m1", file: file)
    #expect(SnoozedAgents.load(from: file).snoozes.isEmpty)
}

@Test func nextMorningIsTheNextEightOClock() throws {
    var calendar = Calendar(identifier: .gregorian)
    calendar.timeZone = try #require(TimeZone(identifier: "America/Toronto"))
    func date(_ year: Int, _ month: Int, _ day: Int, _ hour: Int, _ minute: Int = 0) throws -> Date {
        try #require(calendar.date(from: DateComponents(year: year, month: month, day: day, hour: hour, minute: minute)))
    }
    #expect(SnoozedAgents.nextMorning(after: try date(2026, 10, 13, 15), calendar: calendar) == (try date(2026, 10, 14, 8)))
    #expect(SnoozedAgents.nextMorning(after: try date(2026, 10, 13, 1), calendar: calendar) == (try date(2026, 10, 13, 8)))
    #expect(SnoozedAgents.nextMorning(after: try date(2026, 10, 13, 8), calendar: calendar) == (try date(2026, 10, 14, 8)))
    #expect(SnoozedAgents.nextMorning(after: try date(2026, 12, 31, 23, 59), calendar: calendar) == (try date(2027, 1, 1, 8)))
    let fallBack = SnoozedAgents.nextMorning(after: try date(2026, 10, 31, 22), calendar: calendar)
    #expect(fallBack == (try date(2026, 11, 1, 8)))
    #expect(fallBack.timeIntervalSince(try date(2026, 10, 31, 22)) == 11 * 3600)
    let springForward = SnoozedAgents.nextMorning(after: try date(2027, 3, 14, 1), calendar: calendar)
    #expect(springForward == (try date(2027, 3, 14, 8)))
    #expect(springForward.timeIntervalSince(try date(2027, 3, 14, 1)) == 6 * 3600)
}

@Test func inboxScheduleAddsWakeTimesBetweenMinutes() {
    let schedule = InboxSchedule(wakes: [now.addingTimeInterval(90), now.addingTimeInterval(-10), now.addingTimeInterval(30)])
    let entries = Array(AnySequence { schedule.entries(from: now, mode: .normal) }.prefix(5))
    #expect(entries == [0, 30, 60, 90, 120].map { now.addingTimeInterval($0) })
}

@Test func inboxSectionsFollowStatusAndLastActivity() {
    let hour: UInt64 = 3_600_000
    #expect(InboxSection.of(agent("a", .working, activity: nil), now: now) == .working)
    #expect(InboxSection.of(agent("a", .blocked, activity: nil), now: now) == .needsYou)
    #expect(InboxSection.of(agent("a", .done, activity: nil, since: nowMs - hour), now: now) == .done)
    #expect(InboxSection.of(agent("a", .done, activity: nil, since: nowMs - 25 * hour), now: now) == .inactive)
    #expect(InboxSection.of(agent("a", .done, activity: nowMs - 25 * hour, since: nowMs - hour), now: now) == .done)
    #expect(InboxSection.of(agent("a", .done, activity: nowMs - hour, since: nowMs - 24 * hour), now: now) == .inactive)
    #expect(InboxSection.of(agent("a", .blocked, activity: nowMs - 25 * hour), now: now) == .needsYou)
    #expect(InboxSection.of(agent("a", .idle, activity: nowMs - hour, since: nowMs - 15 * hour), now: now) == .inactive)
    #expect(InboxSection.of(agent("a", .unknown, activity: nowMs - hour), now: now) == .inactive)
    #expect(InboxSection.of(agent("a", .idle, activity: nowMs - 25 * hour), now: now) == .inactive)
    #expect(InboxSection.of(agent("a", .unknown, activity: nil), now: now) == .inactive)
    #expect(InboxSection.of(agent("a", .idle, activity: nil, since: nowMs - hour), now: now) == .inactive)
}

@Test func inboxPutsBlockedInItsOwnSectionThenTheMostRecentAcrossMachines() {
    let entries = [
        entry("m1", [agent("old", .working, activity: 10), agent("blocked", .blocked, activity: 1), agent("idle", .idle, activity: nil)]),
        entry("m2", [agent("new", .working, activity: 30), agent("since", .done, activity: nil, since: nowMs - 50),
                     agent("done", .done, activity: nowMs - 40)]),
    ]
    let items = InboxItem.items(in: entries)
    #expect(items.first { $0.route.terminalId == "new" }?.machine == "Mac m2")
    #expect(items.first { $0.route.terminalId == "old" }?.workspace == "collie")
    let groups = InboxSection.grouped(items, now: now)
    #expect(groups[.needsYou]?.map(\.route.terminalId) == ["blocked"])
    #expect(groups[.working]?.map(\.route.terminalId) == ["new", "old"])
    #expect(groups[.done]?.map(\.route.terminalId) == ["done", "since"])
    #expect(groups[.inactive]?.map(\.route) == [AgentRoute(machineId: "m1", terminalId: "idle")])
}

@MainActor
@Test func contextRingTonesFollowTheOneWarningRule() {
    for left: UInt8 in [100, 41, 21] {
        #expect(ContextRing.tone(left) == .accentColor)
    }
    #expect(ContextRing.tone(255) == ContextRing.tone(100))
    var light = EnvironmentValues()
    light.colorScheme = .light
    var dark = EnvironmentValues()
    dark.colorScheme = .dark
    func onWhite(_ color: Color) -> Double {
        let c = color.resolve(in: light)
        return 1.05 / (0.2126 * Double(c.linearRed) + 0.7152 * Double(c.linearGreen) + 0.0722 * Double(c.linearBlue) + 0.05)
    }
    for (lefts, tone) in [([20, 11] as [UInt8], Color.orange), ([10, 0], AgentState.blocked.color)] {
        for left in lefts {
            #expect(ContextRing.tone(left).resolve(in: dark) == tone.resolve(in: dark))
            #expect(onWhite(ContextRing.tone(left)) >= 4.5)
        }
    }
    #expect(onWhite(.metadata) >= 4.5)
    #expect(onWhite(AgentState.blocked.color) >= 4.5)
    // Dark glass under the approval buttons, about #3D3D3D.
    let red = AgentState.blocked.color.resolve(in: dark)
    #expect((0.2126 * Double(red.linearRed) + 0.7152 * Double(red.linearGreen) + 0.0722 * Double(red.linearBlue) + 0.05) / (0.047 + 0.05) >= 4.5)
}

@Test func theFilterRowEndsBetweenLabels() {
    // Five segments with 14 pt of padding on each side of their labels, after a 16 pt inset.
    let segments: [CGFloat] = [50, 100, 90, 66, 95]
    #expect(InboxFilter.rowWidth(segments: segments, available: 500, atEnd: false) == 500)
    // 378 falls inside the fifth label (336 to 403): the row ends where it starts.
    #expect(InboxFilter.rowWidth(segments: segments, available: 378, atEnd: false) == 336)
    // 330 falls in the padding between the fourth and fifth labels.
    #expect(InboxFilter.rowWidth(segments: segments, available: 330, atEnd: false) == 330)
    // Scrolled to the end of 433 pt, a leading edge at 55 falls between the first two labels; one at 93 would cut the second (80 to 152), so the row starts after it: 433 - 152.
    #expect(InboxFilter.rowWidth(segments: segments, available: 378, atEnd: true) == 378)
    #expect(InboxFilter.rowWidth(segments: segments, available: 340, atEnd: true) == 281)
}
