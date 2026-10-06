import CollieCore
import Foundation
import Testing

@testable import Collie

@Test func coreVersionMatchesCrate() {
    #expect(coreVersion() == "0.1.0")
}

private func agent(_ id: String, _ status: AgentState, since: UInt64, title: String? = nil) -> AgentSummary {
    AgentSummary(
        terminalId: id, workspaceId: "w1", kind: "claude", name: nil, title: title,
        status: status, statusSinceMs: since, cwd: nil, lastLine: nil
    )
}

@Test func blockedAgentsComeFirstLongestWaitingFirst() {
    let sorted = FlockOrder.sorted([
        agent("a", .idle, since: 1),
        agent("b", .blocked, since: 20),
        agent("c", .working, since: 5),
        agent("d", .blocked, since: 10),
        agent("e", .unknown, since: 0),
    ])
    #expect(sorted.map(\.terminalId) == ["d", "b", "c", "a", "e"])
}

@Test func swipingFollowsTheAgentsListAcrossMachinesAndStopsAtEitherEnd() {
    func entry(_ id: String, _ agents: [AgentSummary]) -> MachineFlockEntry {
        let machine = Machine(id: id, label: id, host: "\(id).ts.net", port: 8457, nodeId: "n\(id)", kind: .mac, key: "")
        let flock = MachineFlock(machine: machine, link: .connected, lastError: nil, details: nil, workspaces: [], agents: agents, approvalsCount: 0)
        return MachineFlockEntry(machine: machine, flock: flock)
    }
    let entries = [
        entry("m1", [agent("idle", .idle, since: 1), agent("blocked", .blocked, since: 5)]),
        entry("m2", []),
        entry("m3", [agent("working", .working, since: 2)]),
    ]
    func route(_ machineId: String, _ terminalId: String) -> AgentRoute { AgentRoute(machineId: machineId, terminalId: terminalId) }
    #expect(FlockOrder.neighbor(of: route("m1", "blocked"), offset: 1, in: entries) == route("m1", "idle"))
    #expect(FlockOrder.neighbor(of: route("m1", "idle"), offset: 1, in: entries) == route("m3", "working"))
    #expect(FlockOrder.neighbor(of: route("m3", "working"), offset: -1, in: entries) == route("m1", "idle"))
    #expect(FlockOrder.neighbor(of: route("m3", "working"), offset: 1, in: entries) == nil)
    #expect(FlockOrder.neighbor(of: route("m1", "blocked"), offset: -1, in: entries) == nil)
    #expect(FlockOrder.neighbor(of: route("m3", "idle"), offset: 1, in: entries) == nil)
}

@Test(arguments: [
    (5.0, "5s"), (59.0, "59s"), (60.0, "1m"), (3599.0, "59m"), (3600.0, "1h 0m"), (3_725.0, "1h 2m"), (90_000.0, "1d"), (-30.0, "0s"),
])
func elapsedFormatting(seconds: Double, expected: String) {
    let since: UInt64 = 1_700_000_000_000
    let now = Date(timeIntervalSince1970: Double(since / 1000) + seconds)
    #expect(Elapsed.string(sinceMs: since, now: now) == expected)
}

@Test(arguments: [
    (59.0, "<1m", "less than a minute"), (60.0, "1m", "1 minute"), (720.0, "12m", "12 minutes"), (3599.0, "59m", "59 minutes"),
    (3600.0, "1h", "1 hour"), (10_799.0, "2h", "2 hours"), (86_399.0, "23h", "23 hours"), (86_400.0, "1d", "1 day"),
    (180_000.0, "2d", "2 days"), (-30.0, "<1m", "less than a minute"),
])
func compactElapsedFormatting(seconds: Double, compact: String, spoken: String) {
    let since: UInt64 = 1_700_000_000_000
    let now = Date(timeIntervalSince1970: Double(since / 1000) + seconds)
    #expect(Elapsed.compact(sinceMs: since, now: now) == compact)
    #expect(Elapsed.spoken(sinceMs: since, now: now) == spoken)
}

@Test func statusAgeReadsInFull() {
    let since: UInt64 = 1_700_000_000_000
    let now = Date(timeIntervalSince1970: Double(since / 1000) + 720)
    #expect(Elapsed.spoken(.working, sinceMs: since, now: now) == "working for 12 minutes")
    #expect(Elapsed.spoken(.blocked, sinceMs: since, now: now) == "blocked for 12 minutes")
    #expect(Elapsed.spoken(.idle, sinceMs: since, now: now) == "idle for 12 minutes")
    #expect(Elapsed.spoken(.done, sinceMs: since, now: now) == "done 12 minutes ago")
}

@Test func titleFallsBackToNameKindThenTerminal() {
    #expect(agent("t1", .idle, since: 0, title: "fix the build").displayTitle == "fix the build")
    #expect(agent("t1", .idle, since: 0, title: "  ").displayTitle == "claude")
    let bare = AgentSummary(
        terminalId: "t9", workspaceId: "w", kind: nil, name: nil, title: nil,
        status: .idle, statusSinceMs: 0, cwd: nil, lastLine: nil
    )
    #expect(bare.displayTitle == "t9")
}

private func node(_ state: TailnetState, authUrl: String? = nil) -> NodeState {
    NodeState(backendState: state, authUrl: authUrl, selfDnsName: nil, loginName: nil)
}

@Test func onboardingStatusGuidesMachineAuth() {
    let status = OnboardingStatus(node: node(.needsMachineAuth), signingIn: false, error: nil)
    #expect(status?.message.contains("approve") == true)
    #expect(status?.isError == false)
    #expect(OnboardingStatus(node: node(.notStarted), signingIn: false, error: nil) == nil)
    #expect(OnboardingStatus(node: node(.needsLogin), signingIn: true, error: nil)?.message.contains("signing in") == true)
    #expect(OnboardingStatus(node: nil, signingIn: false, error: "boom") == OnboardingStatus(message: "boom", isError: true))
}

@MainActor
@Test func pairingRequiresInviteAndLabel() {
    let model = PairingModel()
    #expect(!model.canPair)
    model.invite = "https://example.com"
    #expect(!model.canPair)
    model.invite = "collie://pair#v=1&h=mac.ts.net&p=8457&n=nMAC&c=Zm9vYmFyYmF6cXV4cXV1dQ"
    #expect(model.canPair)
    model.deviceLabel = "   "
    #expect(!model.canPair)
    model.deviceLabel = " Richard's iPhone "
    #expect(model.trimmedLabel == "Richard's iPhone")
    #expect(model.canPair)
}

@Test func coreErrorsDescribeThemselves() {
    let error: any Error = CoreError.PinViolation(message: "mac is not tagged tag:collie-mac")
    #expect(describe(error) == "refusing to connect: mac is not tagged tag:collie-mac")
}
