import CollieCore
import Foundation
import Synchronization
import SwiftUI
import Testing

@testable import Collie

private let now = Date(timeIntervalSince1970: 2_000_000_000)
private let nowMs = UInt64(2_000_000_000_000)

private let mac = Machine(id: "m1", label: "Mac", host: "mac.ts.net", port: 8457, nodeId: "nMAC", kind: .mac, key: "")

private func approval(
    _ id: String, options: [ApprovalDecision] = [.approve, .approveAlways, .deny], choices: [ApprovalChoice] = [],
    acceptsInput: Bool = false, toolSummary: String = "ls", snippet: String = "Do you want to proceed?",
    agent: String = "claude", workspace: String = "collie", expiresAtMs: UInt64 = .max
) -> PendingApproval {
    PendingApproval(
        approvalId: id, terminalId: "term_1", agentLabel: agent, workspaceLabel: workspace, snippet: snippet,
        toolName: "Bash", toolSummary: toolSummary, options: options, choices: choices, acceptsInput: acceptsInput,
        hasTextField: false, supportsNote: false, createdAtMs: 1, expiresAtMs: expiresAtMs
    )
}

private func item(_ approval: PendingApproval, machine: Machine = mac) -> ApprovalItem {
    ApprovalItem(machine: machine, approval: approval, link: .connected)
}

private func agent(_ id: String, _ status: AgentState, activity: UInt64?, line: String? = nil) -> AgentSummary {
    AgentSummary(
        terminalId: id, workspaceId: "w1", kind: "claude", name: nil, title: nil,
        status: status, statusSinceMs: 0, cwd: nil, lastLine: line, contextLeft: 40, lastActivityMs: activity
    )
}

private func entry(
    _ machine: Machine, _ agents: [AgentSummary], usage: PlanUsage? = nil, workspace: String = "collie", link: LinkPhase = .connected
) -> MachineFlockEntry {
    let workspaces = [WorkspaceSummary(workspaceId: "w1", label: workspace, number: 1, status: .idle, cwd: nil)]
    let flock = MachineFlock(
        machine: machine, link: link, lastError: nil, details: nil, workspaces: workspaces, agents: agents,
        approvalsCount: 0, planUsage: usage
    )
    return MachineFlockEntry(machine: machine, flock: flock)
}

private let menu = [ApprovalChoice(index: 0, label: "Yes", current: true), ApprovalChoice(index: 1, label: "No", current: false)]

private func shown(_ approvals: [WatchApproval]) -> WatchState {
    WatchState(approvals: approvals, agents: [], usage: nil, decisionsAllowed: true, live: true)
}

@Test func watchStateCarriesApprovalsAsThePhoneShowsThem() throws {
    let items = [
        item(approval("ap_1")),
        item(approval("ap_menu", options: [], choices: menu, acceptsInput: true)),
        item(approval("ap_terminal", options: [])),
        item(approval("ap_expired", expiresAtMs: nowMs)),
    ]
    let state = WatchState(items: items, entries: [], allowed: false, live: true, now: now)
    #expect(state.approvals.map(\.id) == ["ap_1", "ap_menu", "ap_terminal"])
    #expect(!state.decisionsAllowed)
    let first = state.approvals[0]
    #expect(first.nodeId == "nMAC")
    #expect(first.command == "Bash: ls")
    #expect(first.place == "collie · Mac")
    #expect(first.options == [.approve, .deny])
    #expect(state.approvals[1].options.isEmpty)
    #expect(state.approvals[1].choices == [WatchChoice(index: 0, label: "Yes"), WatchChoice(index: 1, label: "No")])
    #expect(!state.approvals[1].answeredInTerminal)
    #expect(state.approvals[2].answeredInTerminal)

    let long = String(repeating: "x", count: 1000)
    let many = (0..<12).map { item(approval("ap_\($0)", toolSummary: long, snippet: long, agent: long, workspace: long)) }
    let capped = WatchState(items: many, entries: [], allowed: true, live: true, now: now)
    #expect(capped.approvals.count == WatchState.maxApprovals)
    let clipped = try #require(capped.approvals.first)
    #expect(clipped.command?.count == 200)
    #expect(clipped.snippet.count == 400)
    #expect(clipped.agent.count == 60)
    #expect(clipped.place.count == 60)
}

@Test func watchStateGroupsAgentsLikeTheInbox() {
    let entries = [
        entry(mac, [
            agent("old", .working, activity: nowMs - 10_000, line: "Running tests"),
            agent("blocked", .blocked, activity: nowMs - 50_000),
            agent("done", .done, activity: nowMs - 5_000),
            agent("archived", .idle, activity: nowMs - 48 * 3_600_000),
        ])
    ]
    let state = WatchState(items: [], entries: entries, allowed: true, live: true, now: now)
    #expect(state.agents.map(\.id) == ["m1/blocked", "m1/old", "m1/done"])
    #expect(state.agents.map(\.done) == [false, false, true])
    #expect(state.agents.map(\.status) == [.blocked, .working, .done])
    #expect(state.agents[1].title == "Running tests")
    #expect(state.agents[0].title == "claude")
    #expect(state.agents[0].workspace == "collie")
    #expect(state.agents[0].machine == "Mac")
    #expect(state.agents[0].contextLeft == 40)

    let crowd = [entry(mac, (0..<50).map { agent("t\($0)", .working, activity: nowMs - UInt64($0)) })]
    #expect(WatchState(items: [], entries: crowd, allowed: true, live: true, now: now).agents.count == WatchState.maxAgents)
}

@Test func watchStateTakesTheNewestPlanUsage() throws {
    let linux = Machine(id: "m2", label: "omarchy", host: "omarchy.ts.net", port: 8457, nodeId: "nLINUX", kind: .linux, key: "")
    let older = PlanUsage(fiveHour: UsageWindow(usedPercent: 10, resetsAtMs: nowMs + 60_000), sevenDay: nil, recordedMs: nowMs - 60_000)
    let newer = PlanUsage(fiveHour: UsageWindow(usedPercent: 73, resetsAtMs: nowMs + 60_000), sevenDay: nil, recordedMs: nowMs - 1_000)
    let state = WatchState(items: [], entries: [entry(mac, [], usage: older), entry(linux, [], usage: newer)], allowed: true, live: false, now: now)
    let usage = try #require(state.usage)
    #expect(usage == WatchUsage(fiveHourUsed: 73, fiveHourResetsAtMs: nowMs + 60_000, claudeRecordedMs: newer.recordedMs))
    #expect(usage.windows(now: now)[0]?.used == 73)
    #expect(usage.windows(now: now.addingTimeInterval(60))[0]?.used == nil)
    #expect(WatchUsage(fiveHourUsed: nil, fiveHourResetsAtMs: nil).windows(now: now)[0]?.used == nil)
    #expect(WatchState(items: [], entries: [entry(mac, [])], allowed: true, live: true, now: now).usage == nil)
}

@Test func watchComplicationExpiresEachRingIndependently() throws {
    let usage = WatchUsage(
        fiveHourUsed: 29, fiveHourResetsAtMs: nowMs + 9_000_000,
        sevenDayUsed: 73, sevenDayResetsAtMs: nowMs + 86_400_000,
        codexUsed: 90, codexResetsAtMs: nowMs + 86_400_000
    )
    let windows = usage.windows(now: now)
    #expect(windows.map { $0?.used } == [29, 73, 90])
    #expect(try #require(windows[0]).interval.upperBound.timeIntervalSince(now) == 9_000)
    #expect(try #require(windows[0]).interval.upperBound.timeIntervalSince(windows[0]!.interval.lowerBound) == 18_000)
    #expect(try #require(windows[1]).interval.upperBound.timeIntervalSince(windows[1]!.interval.lowerBound) == 604_800)
    #expect([0, 9_000, 86_400].allSatisfy { usage.timelineDates(now: now).contains(now.addingTimeInterval($0)) })
    #expect(usage.windows(now: now.addingTimeInterval(9_000)).map { $0?.used } == [nil, 73, 90])
    #expect(usage.windows(now: now.addingTimeInterval(86_400)).allSatisfy { $0 == nil })
    #expect(usage.timelineDates(now: now.addingTimeInterval(86_400)) == [now.addingTimeInterval(86_400)])
    #expect(WatchUsage(fiveHourUsed: nil, fiveHourResetsAtMs: nowMs + 60_000).timelineDates(now: now) == [now])
}

@Test func watchUsageColorsBlendFromBrightGreenToDeepRed() {
    let environment = EnvironmentValues()
    let colors = [0, 60, 80, 90, 98, 100].map { WatchUsage.usedColor(UInt8($0)).resolve(in: environment) }
    #expect(colors[0].green > 0.95 && colors[0].red < 0.2)
    #expect(colors[1].red > 0.95 && colors[1].green > 0.8 && colors[1].blue < 0.05)
    #expect(colors[2].red > 0.95 && colors[2].green > 0.4 && colors[2].green < 0.5)
    #expect(colors[3].red > 0.95 && colors[3].green < 0.2)
    #expect(colors[4].red < colors[3].red && colors[4].red > 0.6 && colors[4].green < colors[3].green)
    #expect(WatchUsage.usedColor(255).resolve(in: environment) == colors[5])
    for percent in 1...100 {
        let previous = WatchUsage.usedColor(UInt8(percent - 1)).resolve(in: environment)
        let current = WatchUsage.usedColor(UInt8(percent)).resolve(in: environment)
        #expect(abs(current.red - previous.red) < 0.1)
        #expect(abs(current.green - previous.green) < 0.1)
        #expect(abs(current.blue - previous.blue) < 0.1)
    }
}

@Test func watchCodexRingUsesCalendarMonths() throws {
    for (reset, start, days) in [
        ("2026-11-01T00:00:00Z", "2026-10-01T00:00:00Z", 31),
        ("2026-03-01T00:00:00Z", "2026-02-01T00:00:00Z", 28),
        ("2028-03-01T00:00:00Z", "2028-02-01T00:00:00Z", 29),
        ("2027-01-01T00:00:00Z", "2026-12-01T00:00:00Z", 31),
    ] {
        let end = try #require(ISO8601DateFormatter().date(from: reset))
        let beginning = try #require(ISO8601DateFormatter().date(from: start))
        let usage = WatchUsage(fiveHourUsed: nil, fiveHourResetsAtMs: nil, codexUsed: 42, codexResetsAtMs: end.unixMs)
        let window = try #require(usage.windows(now: beginning)[2])
        #expect(window.interval == beginning...end)
        #expect(window.interval.upperBound.timeIntervalSince(window.interval.lowerBound) == Double(days * 86_400))
        #expect(usage.windows(now: end)[2] == nil)
    }
}

@Test func watchUsageDecodesOlderCachedReadings() throws {
    let data = Data(#"{"fiveHourUsed":42,"fiveHourResetsAtMs":2000000060000}"#.utf8)
    let usage = try JSONDecoder().decode(WatchUsage.self, from: data)
    #expect(usage.windows(now: now).map { $0?.used } == [42, nil, nil])
    #expect(try JSONDecoder().decode(WatchUsage.self, from: JSONEncoder().encode(usage)) == usage)
}

@Test func watchReceivesWeeklyAndCodexUsageWithoutFiveHourUsage() throws {
    let reset = nowMs + 86_400_000
    for used in [UInt64(1532), 10_001] {
        let plan = PlanUsage(
            fiveHour: nil, sevenDay: UsageWindow(usedPercent: 150, resetsAtMs: reset), recordedMs: nowMs,
            codex: CodexUsage(used: used, limit: 10_000, resetsAtMs: reset, recordedMs: nowMs)
        )
        let foreground = WatchState(items: [], entries: [entry(mac, [], usage: plan)], allowed: false, live: true, now: now)
        let background = WatchState.refreshed(
            nil, listed: [MachineApprovals(machineId: mac.id, approvals: [], planUsage: plan)], machines: [mac], allowed: false, now: now
        )
        let usage = try #require(foreground.usage)
        #expect(background.usage == usage)
        #expect(usage.windows(now: now).map { $0?.used } == [nil, 100, used > 10_000 ? 100 : 15])
        #expect(try JSONDecoder().decode(WatchUsage.self, from: JSONEncoder().encode(usage)) == usage)
    }
    let codexOnly = PlanUsage(
        fiveHour: nil, sevenDay: nil, recordedMs: nowMs,
        codex: CodexUsage(used: 42, limit: 100, resetsAtMs: reset, recordedMs: nowMs)
    )
    #expect(WatchState(items: [], entries: [entry(mac, [], usage: codexOnly)], allowed: false, live: true, now: now).usage?.codexUsed == 42)
    let invalid = PlanUsage(
        fiveHour: nil, sevenDay: nil, recordedMs: nowMs,
        codex: CodexUsage(used: 42, limit: 0, resetsAtMs: reset, recordedMs: nowMs)
    )
    #expect(WatchUsage(invalid).windows(now: now).allSatisfy { $0 == nil })
}

@Test func watchKeepsClaudeWhenAnotherMachineOnlyHasCodex() throws {
    let linux = Machine(id: "m2", label: "Linux", host: "linux.ts.net", port: 8457, nodeId: "nLINUX", kind: .linux, key: "")
    let reset = nowMs + 86_400_000
    let claude = PlanUsage(
        fiveHour: UsageWindow(usedPercent: 42, resetsAtMs: reset), sevenDay: UsageWindow(usedPercent: 73, resetsAtMs: reset), recordedMs: nowMs,
        codex: CodexUsage(used: 10, limit: 100, resetsAtMs: reset, recordedMs: nowMs - 1_000)
    )
    let codex = PlanUsage(
        fiveHour: nil, sevenDay: nil, recordedMs: nowMs + 1_000,
        codex: CodexUsage(used: 90, limit: 100, resetsAtMs: reset, recordedMs: nowMs - 500)
    )
    let state = WatchState(items: [], entries: [entry(mac, [], usage: claude), entry(linux, [], usage: codex)], allowed: false, live: true, now: now)
    let usage = try #require(state.usage)
    #expect(usage.windows(now: now).map { $0?.used } == [42, 73, 90])
    let refreshed = WatchState.refreshed(
        nil, listed: [MachineApprovals(machineId: mac.id, approvals: [], planUsage: claude), MachineApprovals(machineId: linux.id, approvals: [], planUsage: codex)],
        machines: [mac, linux], allowed: false, now: now
    )
    #expect(refreshed.usage == usage)
    #expect(usage.claudeRecordedMs == claude.recordedMs)
    #expect(usage.codexRecordedMs == codex.codex?.recordedMs)
    let cleared = WatchState.refreshed(state, listed: [MachineApprovals(machineId: mac.id, approvals: [])], machines: [mac], allowed: false, now: now)
    #expect(cleared.usage == nil)
}

@Test func watchGetsAgentChangesAloneOnlyWhileItsAppIsReachable() {
    let shown = WatchState(items: [], entries: [entry(mac, [agent("t1", .working, activity: nowMs)])], allowed: true, live: true, now: now)
    var agents = shown
    agents.agents = WatchState(
        items: [], entries: [entry(mac, [agent("t1", .working, activity: nowMs, line: "Running tests")])], allowed: true, live: true, now: now
    ).agents
    #expect(agents.agents.count == 1 && agents != shown)
    var approvals = agents
    approvals.approvals = [WatchApproval(item: item(approval("ap_1")))]
    let later = now.addingTimeInterval(60)
    #expect(WatchLink.due(agents, lastSent: nil, reachable: false, now: now))
    #expect(!WatchLink.due(shown, lastSent: (shown, now), reachable: true, now: later))
    #expect(!WatchLink.due(agents, lastSent: (shown, now), reachable: false, now: later.addingTimeInterval(3_600)))
    #expect(!WatchLink.due(agents, lastSent: (shown, now), reachable: true, now: later.addingTimeInterval(-1)))
    #expect(WatchLink.due(agents, lastSent: (shown, now), reachable: true, now: later))
    for reachable in [false, true] {
        #expect(WatchLink.due(approvals, lastSent: (shown, now), reachable: reachable, now: now), "approvals go at once")
        var off = shown
        off.decisionsAllowed = false
        #expect(WatchLink.due(off, lastSent: (shown, now), reachable: reachable, now: now), "the setting goes at once")
        var usage = shown
        usage.usage = WatchUsage(fiveHourUsed: 50, fiveHourResetsAtMs: nowMs + 60_000)
        #expect(WatchLink.due(usage, lastSent: (shown, now), reachable: reachable, now: now), "usage goes at once")
    }
}

@Test func watchDecisionIsRefusedWhileTheSettingIsOff() {
    let state = WatchState(items: [item(approval("ap_1"))], entries: [], allowed: true, live: true, now: now)
    let request = WatchDecisionRequest(nodeId: "nMAC", approvalId: "ap_1", decision: .approve)
    let off = WatchLink.refusal(request, allowed: false, shown: state, now: now)
    #expect(off == "Decisions from Apple Watch are off. Turn them on in collie Settings on the iPhone. Nothing was sent.")
    #expect(WatchLink.refusal(request, allowed: true, shown: state, now: now) == nil)
    // The flag the phone sent the watch is never what allows it.
    #expect(WatchLink.refusal(request, allowed: false, shown: shown(state.approvals), now: now) == off)
}

@Test func watchDecisionNeedsAnApprovalAndAnswerThePhoneShowed() {
    let items = [
        item(approval("ap_1", expiresAtMs: nowMs + 60_000)),
        item(approval("ap_menu", options: [], choices: menu, acceptsInput: true)),
        item(approval("ap_terminal", options: [])),
    ]
    let state = WatchState(items: items, entries: [], allowed: true, live: true, now: now)
    func refused(_ nodeId: String, _ id: String, _ decision: WatchDecision, shown: WatchState? = state, at time: Date = now) -> Bool {
        WatchLink.refusal(WatchDecisionRequest(nodeId: nodeId, approvalId: id, decision: decision), allowed: true, shown: shown, now: time) != nil
    }
    #expect(!refused("nMAC", "ap_1", .approve))
    #expect(!refused("nMAC", "ap_1", .deny))
    #expect(!refused("nMAC", "ap_menu", .choose(1)))
    #expect(refused("nMAC", "ap_1", .approve, shown: nil))
    #expect(refused("nMAC", "ap_unknown", .approve))
    #expect(refused("nLINUX", "ap_1", .approve))
    #expect(refused("nMAC", "ap_1", .choose(0)))
    #expect(refused("nMAC", "ap_menu", .approve))
    #expect(refused("nMAC", "ap_menu", .choose(2)))
    #expect(refused("nMAC", "ap_1", .approve, at: now.addingTimeInterval(60)))
    #expect(refused("nMAC", "ap_terminal", .approve))
    #expect(refused("nMAC", "ap_terminal", .choose(0)))
    #expect(
        WatchLink.refusal(WatchDecisionRequest(nodeId: "nMAC", approvalId: "ap_unknown", decision: .approve), allowed: true, shown: state, now: now)
            == "This approval is no longer pending on the iPhone. Nothing was sent."
    )
}

@Test func watchRefreshListsApprovalsFromEachMacAndKeepsTheSilentOnes() {
    let linux = Machine(id: "m2", label: "omarchy", host: "omarchy.ts.net", port: 8457, nodeId: "nLINUX", kind: .linux, key: "")
    let before = WatchState(
        items: [item(approval("ap_old")), item(approval("ap_linux"), machine: linux), item(approval("ap_gone"), machine: linux)],
        entries: [entry(mac, [agent("t1", .working, activity: nowMs)])], allowed: false, live: true, now: now
    )
    let listed = [
        MachineApprovals(machineId: "m1", approvals: [approval("ap_new"), approval("ap_expired", expiresAtMs: nowMs)]),
        MachineApprovals(machineId: "m2", approvals: nil),
    ]
    let state = WatchState.refreshed(before, listed: listed, machines: [mac, linux], allowed: true, now: now)
    #expect(state.approvals.map(\.id) == ["ap_new", "ap_linux", "ap_gone"])
    #expect(state.approvals[0].command == "Bash: ls")
    #expect(state.agents == before.agents)
    #expect(state.decisionsAllowed)
    #expect(!state.live)
    #expect(WatchLink.refusal(WatchDecisionRequest(nodeId: "nMAC", approvalId: "ap_new", decision: .approve), allowed: true, shown: state, now: now) == nil)
    #expect(WatchLink.refusal(WatchDecisionRequest(nodeId: "nMAC", approvalId: "ap_old", decision: .approve), allowed: true, shown: state, now: now) != nil)

    let off = WatchState.refreshed(nil, listed: [MachineApprovals(machineId: "m1", approvals: [approval("ap_1")])], machines: [mac], allowed: false, now: now)
    #expect(off.approvals.map(\.id) == ["ap_1"])
    #expect(!off.decisionsAllowed)
    #expect(off.agents.isEmpty)
    let unpaired = WatchState.refreshed(before, listed: [MachineApprovals(machineId: "m9", approvals: nil)], machines: [mac], allowed: true, now: now)
    #expect(unpaired.approvals.isEmpty)
}

// #114: in the background the phone's sessions are closed, so a refresh's listing is the only fresh usage.
@Test func watchRefreshTakesTheNewestListedPlanUsage() {
    let linux = Machine(id: "m2", label: "omarchy", host: "omarchy.ts.net", port: 8457, nodeId: "nLINUX", kind: .linux, key: "")
    let resets = nowMs + 3_600_000
    let plan = { (used: UInt8, recorded: UInt64) in
        PlanUsage(fiveHour: UsageWindow(usedPercent: used, resetsAtMs: resets), sevenDay: nil, recordedMs: recorded)
    }
    let shown = WatchState(
        approvals: [], agents: [], usage: WatchUsage(fiveHourUsed: 24, fiveHourResetsAtMs: resets), decisionsAllowed: true, live: true
    )
    let listed = [
        MachineApprovals(machineId: "m1", approvals: [], planUsage: plan(57, nowMs)),
        MachineApprovals(machineId: "m2", approvals: [], planUsage: plan(40, nowMs - 1000)),
    ]
    let state = WatchState.refreshed(shown, listed: listed, machines: [mac, linux], allowed: true, now: now)
    #expect(state.usage == WatchUsage(fiveHourUsed: 57, fiveHourResetsAtMs: resets, claudeRecordedMs: nowMs))
    let silent = WatchState.refreshed(shown, listed: [MachineApprovals(machineId: "m1", approvals: nil)], machines: [mac], allowed: true, now: now)
    #expect(silent.usage == shown.usage)
}

@Test func watchApprovalsChangeOnlyThroughAConnectedLink() {
    let linux = Machine(id: "m2", label: "omarchy", host: "omarchy.ts.net", port: 8457, nodeId: "nLINUX", kind: .linux, key: "")
    // A refresh from a locked phone listed ap_new; the cache, frozen when the sessions closed, predates it.
    let refreshed = WatchState.refreshed(
        nil, listed: [MachineApprovals(machineId: "m1", approvals: [approval("ap_new")])], machines: [mac], allowed: true, now: now
    )
    let stale = [item(approval("ap_old"))]
    for link in [LinkPhase.connecting, .waiting, .offline] {
        let state = WatchState.published(refreshed, items: stale, entries: [entry(mac, [], link: link)], allowed: true, live: false, now: now)
        #expect(state.approvals.map(\.id) == ["ap_new"])
        #expect(WatchLink.refusal(WatchDecisionRequest(nodeId: "nMAC", approvalId: "ap_new", decision: .approve), allowed: true, shown: state, now: now) == nil)
    }

    let entries = [entry(mac, [agent("t1", .working, activity: nowMs)]), entry(linux, [], link: .connecting)]
    let before = WatchState(items: [item(approval("ap_linux"), machine: linux)], entries: [], allowed: true, live: true, now: now)
    let state = WatchState.published(before, items: [item(approval("ap_mac"))], entries: entries, allowed: false, live: true, now: now)
    #expect(state.approvals.map(\.id) == ["ap_mac", "ap_linux"])
    #expect(state.agents.map(\.id) == ["m1/t1"])
    #expect(!state.decisionsAllowed)
    #expect(state.live)
    let cleared = WatchState.published(state, items: [], entries: [entry(mac, []), entry(linux, [])], allowed: true, live: true, now: now)
    #expect(cleared.approvals.isEmpty)
}

@Test func watchDecisionsMapToCoreDecisions() {
    #expect(WatchDecision.approve.core == .approve)
    #expect(WatchDecision.deny.core == .deny)
    #expect(WatchDecision.choose(2).core == .choose(choice: 2))
    let all: [WatchDecision] = [.approve, .deny] + (0...UInt8.max).map { .choose($0) }
    #expect(!all.contains { $0.core == .approveAlways })
}

@Test func watchStateFitsInAnApplicationContext() throws {
    let wide = String(repeating: "𝄞", count: 2000)
    let choices = (0..<20).map { ApprovalChoice(index: UInt8($0), label: wide, current: false) }
    let items = (0..<20).map {
        item(approval("ap_\($0)", options: [], choices: choices, toolSummary: wide, snippet: wide, agent: wide, workspace: wide))
    }
    let machine = Machine(id: "m1", label: wide, host: "mac.ts.net", port: 8457, nodeId: "nMAC", kind: .mac, key: "")
    let agents = (0..<200).map { agent("term_\($0)", .working, activity: nowMs, line: wide) }
    let usage = PlanUsage(fiveHour: UsageWindow(usedPercent: 100, resetsAtMs: .max), sevenDay: nil, recordedMs: .max)
    let state = WatchState(
        items: items.map { ApprovalItem(machine: machine, approval: $0.approval, link: .connected) },
        entries: [entry(machine, agents, usage: usage, workspace: wide)], allowed: true, live: true, now: now
    )
    #expect(state.approvals.count == WatchState.maxApprovals)
    #expect(state.agents.count == WatchState.maxAgents)
    #expect(state.approvals[0].choices.count == 9)
    let encoder = JSONEncoder()
    encoder.outputFormatting = .sortedKeys
    #expect(try encoder.encode(state).count < 65_536)
}

@Test func watchKeepsTheButtonsWhenNothingReachedCollied() {
    #expect(BackgroundOutcome.applied(decision: .approve).watchAnswered)
    #expect(BackgroundOutcome.notFound.watchAnswered)
    #expect(BackgroundOutcome.unreachable(stage: .decide, message: "").watchAnswered)
    #expect(!BackgroundOutcome.unreachable(stage: .connect, message: "").watchAnswered)
    #expect(!BackgroundOutcome.unauthorized(message: "").watchAnswered)
    #expect(!BackgroundOutcome.failed(message: "").watchAnswered)
}

@Test func devicePrefsKeepWatchDecisionsOffByDefault() throws {
    #expect(!DevicePrefs().watchDecisions)
    #expect(try !JSONDecoder().decode(DevicePrefs.self, from: Data("{}".utf8)).watchDecisions)
    #expect(try !JSONDecoder().decode(DevicePrefs.self, from: Data(#"{"wrapLines":false,"fontSize":13}"#.utf8)).watchDecisions)
    #expect(DevicePrefs.load(from: nil).watchDecisions == false)
    var prefs = DevicePrefs()
    prefs.watchDecisions = true
    #expect(try JSONDecoder().decode(DevicePrefs.self, from: JSONEncoder().encode(prefs)).watchDecisions)
}

@Test func watchChecksADecisionBeforeAskingForTheWrist() throws {
    let items = [item(approval("ap_1", expiresAtMs: nowMs + 60_000)), item(approval("ap_menu", options: [], choices: menu, acceptsInput: true))]
    let state = WatchState(items: items, entries: [], allowed: true, live: true, now: now)
    let approval = try #require(state.approvals.first { $0.id == "ap_1" })
    let menuApproval = try #require(state.approvals.first { $0.id == "ap_menu" })
    #expect(approval.canDecide(.approve, allowed: true, answered: [], now: now))
    #expect(approval.canDecide(.deny, allowed: true, answered: ["ap_other"], now: now))
    #expect(!approval.canDecide(.approve, allowed: false, answered: [], now: now))
    #expect(!approval.canDecide(.approve, allowed: true, answered: ["ap_1"], now: now))
    #expect(!approval.canDecide(.choose(1), allowed: true, answered: [], now: now))
    #expect(!approval.canDecide(.approve, allowed: true, answered: [], now: now.addingTimeInterval(60)))
    #expect(menuApproval.canDecide(.choose(1), allowed: true, answered: [], now: now))
    #expect(!menuApproval.canDecide(.approve, allowed: true, answered: [], now: now))
}

private func prefsFile() throws -> URL {
    let dir = FileManager.default.temporaryDirectory.appending(path: UUID().uuidString, directoryHint: .isDirectory)
    try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
    return dir.appending(path: "prefs.json")
}

@MainActor
@Test func watchDecisionsNeedTheOwnerOnlyToTurnOn() async throws {
    let file = try prefsFile()
    defer { try? FileManager.default.removeItem(at: file.deletingLastPathComponent()) }
    let auth = FakeAuthenticator()
    auth.state.withLock { $0.result = false }
    #expect(await DevicePrefs.setWatchDecisions(true, in: file, auth: auth) == nil)
    #expect(!DevicePrefs.load(from: file).watchDecisions)
    auth.state.withLock { $0.result = true }
    #expect(await DevicePrefs.setWatchDecisions(true, in: file, auth: auth) == true)
    #expect(DevicePrefs.load(from: file).watchDecisions)
    #expect(auth.state.withLock { $0.reasons } == ["Allow decisions from Apple Watch", "Allow decisions from Apple Watch"])
    auth.state.withLock { $0.result = false }
    #expect(await DevicePrefs.setWatchDecisions(false, in: file, auth: auth) == false)
    #expect(!DevicePrefs.load(from: file).watchDecisions)
    #expect(auth.state.withLock { $0.reasons }.count == 2)
}

@Test func anotherWatchTurnsWatchDecisionsOff() throws {
    let file = try prefsFile()
    defer { try? FileManager.default.removeItem(at: file.deletingLastPathComponent()) }
    DevicePrefs(wrapLines: false, watchDecisions: true).save(to: file)
    DevicePrefs.turnOffWatchDecisions(in: file)
    #expect(DevicePrefs.load(from: file) == DevicePrefs(wrapLines: false))
    try FileManager.default.removeItem(at: file)
    DevicePrefs.turnOffWatchDecisions(in: file)
    #expect(!FileManager.default.fileExists(atPath: file.path))
}

@Test func watchPaceMarkersCompareUsedAllowanceWithElapsedTime() {
    let interval = now...now.addingTimeInterval(3600)
    for used in [UInt8(0), 25, 50, 75, 98, 100, 255] {
        let window = WatchUsage.Window(used: used, interval: interval)
        #expect(window.usedFraction == Double(min(used, 100)) / 100)
        #expect(window.elapsed(now: now.addingTimeInterval(-1)) == 0)
        #expect(window.elapsed(now: now) == 0)
        #expect(window.elapsed(now: now.addingTimeInterval(1800)) == 0.5)
        #expect(window.elapsed(now: now.addingTimeInterval(3600)) == 1)
        #expect(window.elapsed(now: now.addingTimeInterval(3601)) == 1)
    }
    let weekly = WatchUsage.Window(used: 98, interval: now.addingTimeInterval(-4.5 * 86400)...now.addingTimeInterval(2.5 * 86400))
    #expect(weekly.usedFraction == 0.98)
    #expect(abs(weekly.elapsed(now: now) - 4.5 / 7) < 0.000001)
    #expect(WatchUsage.Window(used: 50, interval: now...now).elapsed(now: now) == 0)
}

@Test func watchUsageBecomesStaleWithoutAnotherPhoneUpdate() throws {
    let usage = WatchUsage(
        fiveHourUsed: 0, fiveHourResetsAtMs: nowMs + 7_200_000,
        sevenDayUsed: 98, sevenDayResetsAtMs: nowMs + 86_400_000,
        codexUsed: 38, codexResetsAtMs: nowMs + 86_400_000,
        claudeRecordedMs: nowMs, codexRecordedMs: nowMs - 600_000
    )
    let windows = usage.windows(now: now)
    #expect(windows.map { $0?.isStale(now: now) } == [false, false, false])
    #expect(windows.map { $0?.isStale(now: now.addingTimeInterval(1200)) } == [false, false, true])
    #expect(windows.map { $0?.isStale(now: now.addingTimeInterval(1800)) } == [true, true, true])
    #expect([0, 1200, 1800, 7200, 86400].allSatisfy { usage.timelineDates(now: now).contains(now.addingTimeInterval($0)) })
    let later = usage.timelineDates(now: now.addingTimeInterval(1800))
    #expect([1800, 7200, 86400].allSatisfy { later.contains(now.addingTimeInterval($0)) })
    #expect(!later.contains(now.addingTimeInterval(1200)))
    #expect(windows[0]?.used == 0)
    #expect(usage.windows(now: now.addingTimeInterval(7200))[0] == nil)
    let legacy = WatchUsage(fiveHourUsed: 42, fiveHourResetsAtMs: nowMs + 60_000)
    #expect(try #require(legacy.windows(now: now)[0]).isStale(now: now))
    #expect(legacy.timelineDates(now: now) == [now, now.addingTimeInterval(60)])
    let skewed = WatchUsage(fiveHourUsed: 42, fiveHourResetsAtMs: nowMs + 60_000, claudeRecordedMs: nowMs + 30_000)
    #expect(try #require(skewed.windows(now: now)[0]).isStale(now: now) == false)
    #expect(skewed.timelineDates(now: now) == [now, now.addingTimeInterval(60)])
    #expect(try JSONDecoder().decode(WatchUsage.self, from: JSONEncoder().encode(usage)) == usage)
}

@Test func watchPaceTimelineAdvancesWithoutFreshUsageAndStaysBounded() {
    let usage = WatchUsage(
        fiveHourUsed: 0, fiveHourResetsAtMs: nowMs + 3_650_000,
        sevenDayUsed: 98, sevenDayResetsAtMs: nowMs + 7 * 86_400_000,
        codexUsed: 42, codexResetsAtMs: nowMs + 28 * 86_400_000,
        claudeRecordedMs: nowMs - 1_000, codexRecordedMs: nowMs
    )
    let dates = usage.timelineDates(now: now)
    #expect(dates.first == now)
    #expect(dates.last == now.addingTimeInterval(86400))
    #expect(dates.count <= 292)
    #expect(dates == Array(Set(dates)).sorted())
    #expect(dates.contains(now.addingTimeInterval(3650)))
    #expect(dates.contains(now.addingTimeInterval(1799)))
    #expect(zip(dates, dates.dropFirst()).allSatisfy { $1.timeIntervalSince($0) <= 300 })
    let tomorrow = usage.timelineDates(now: now.addingTimeInterval(86400))
    #expect(tomorrow.first == now.addingTimeInterval(86400))
    #expect(tomorrow.last == now.addingTimeInterval(2 * 86400))
    #expect(usage.windows(now: tomorrow[0])[0] == nil)
}
