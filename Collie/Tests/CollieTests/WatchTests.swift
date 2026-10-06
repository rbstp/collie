import CollieCore
import Foundation
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

private func entry(_ machine: Machine, _ agents: [AgentSummary], usage: PlanUsage? = nil, workspace: String = "collie") -> MachineFlockEntry {
    let workspaces = [WorkspaceSummary(workspaceId: "w1", label: workspace, number: 1, status: .idle, cwd: nil)]
    let flock = MachineFlock(
        machine: machine, link: .connected, lastError: nil, details: nil, workspaces: workspaces, agents: agents,
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
    #expect(usage == WatchUsage(fiveHourUsed: 73, fiveHourResetsAtMs: nowMs + 60_000))
    #expect(usage.fiveHour(now: now) == 73)
    #expect(usage.fiveHour(now: now.addingTimeInterval(60)) == nil)
    #expect(WatchUsage(fiveHourUsed: nil, fiveHourResetsAtMs: nil).fiveHour(now: now) == nil)
    #expect(WatchState(items: [], entries: [entry(mac, [])], allowed: true, live: true, now: now).usage == nil)
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
