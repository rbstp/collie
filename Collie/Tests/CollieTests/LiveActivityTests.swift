import AppIntents
import CollieCore
import CryptoKit
import Foundation
import Testing

@testable import Collie

/// docs/protocol/live-activity-content-state.json, shared with collied's push tests.
private func fixture() throws -> [String: Any] {
    let url = URL(filePath: #filePath).deletingLastPathComponent()
        .appending(path: "../../../docs/protocol/live-activity-content-state.json")
    return try #require(try JSONSerialization.jsonObject(with: Data(contentsOf: url)) as? [String: Any])
}

@Test func contentStateDecodesWhatColliedSends() throws {
    let object = try fixture()
    let json = try JSONSerialization.data(withJSONObject: try #require(object["content_state"]))
    let state = try JSONDecoder().decode(AgentActivityAttributes.ContentState.self, from: json)
    let unix = try #require(object["status_since_unix"] as? Double)
    #expect(state.status == .blocked)
    #expect(state.statusSince == Date(timeIntervalSince1970: unix))
    #expect(state.title == "api-fixer")
    #expect(state.kind == "claude")
    #expect(state.workspace == "api")
    #expect(state.approvals == 1)
    #expect(state.approvalId == nil)
    #expect(state.enc == nil)
    #expect(state.pendingApproval == nil)
    #expect(state.command(key: vectorKey) == nil)
}

@Test func contentStateFromAnOlderColliedHasNoKind() throws {
    let object = try #require(try fixture()["content_state"] as? [String: Any])
    let older = object.filter { $0.key != "kind" }
    let state = try JSONDecoder().decode(
        AgentActivityAttributes.ContentState.self, from: JSONSerialization.data(withJSONObject: older)
    )
    #expect(state.kind == nil)
    #expect(state.title == "api-fixer")
    #expect(try JSONSerialization.jsonObject(with: JSONEncoder().encode(state)) as? NSDictionary == older as NSDictionary)
}

@Test func kindsMatchCollied() throws {
    #expect(try fixture()["kinds"] as? [String] == AgentActivityAttributes.ContentState.kinds)
}

@Test func theAgentMostInNeedIsTheMostRelevant() {
    let ranked = [AgentActivityStatus.blocked, .done, .working, .idle, .unknown].map {
        AgentActivityAttributes.ContentState(status: $0, statusSince: .now, title: "a", workspace: nil, approvals: 0).relevance
    }
    #expect(zip(ranked, ranked.dropFirst()).allSatisfy { $0 > $1 })
}

@Test func contentStateEncodesTheSameJSON() throws {
    let object = try fixture()
    let expected = try #require(object["content_state"] as? NSDictionary)
    let state = try JSONDecoder().decode(
        AgentActivityAttributes.ContentState.self, from: JSONSerialization.data(withJSONObject: expected)
    )
    let encoded = try JSONEncoder().encode(state)
    #expect(try JSONSerialization.jsonObject(with: encoded) as? NSDictionary == expected)
    #expect(try JSONDecoder().decode(AgentActivityAttributes.ContentState.self, from: encoded) == state)
}

@Test func everyStatusDecodesAndUnknownOnesFallBack() throws {
    let statuses = try #require(try fixture()["statuses"] as? [String])
    for status in statuses {
        let decoded = try JSONDecoder().decode(AgentActivityStatus.self, from: Data("\"\(status)\"".utf8))
        #expect(decoded.rawValue == status)
    }
    #expect(try JSONDecoder().decode(AgentActivityStatus.self, from: Data(#""sleeping""#.utf8)) == .unknown)
}

@Test func contentStateWithApprovalOpensWithTheMacKey() throws {
    let object = try fixture()
    let expected = try #require(object["content_state_with_approval"] as? NSDictionary)
    let state = try JSONDecoder().decode(
        AgentActivityAttributes.ContentState.self, from: JSONSerialization.data(withJSONObject: expected)
    )
    let approvalId = try #require(state.approvalId)
    #expect(state.status == .blocked)
    #expect(state.pendingApproval == approvalId)
    #expect(state.enc != nil)
    #expect(state.command(key: vectorKey) == "Bash: echo hi")
    #expect(state.command(key: SymmetricKey(size: .bits256)) == nil)
    #expect(state.command(key: nil) == nil)
    var other = state
    other.approvalId = "ap_other"
    #expect(other.command(key: vectorKey) == nil)
    var working = state
    working.status = .working
    #expect(working.pendingApproval == nil)
    #expect(working.command(key: vectorKey) == nil)
    let encoded = try JSONEncoder().encode(state)
    #expect(try JSONSerialization.jsonObject(with: encoded) as? NSDictionary == expected)
}

@Test func localUpdatesKeepTheApprovalOnlyWhileBlocked() throws {
    let since = Date(timeIntervalSinceReferenceDate: 812_721_600)
    var pushed = AgentActivityAttributes.ContentState(
        status: .blocked, statusSince: since, title: "claude", workspace: "api", approvals: 1, approvalId: "ap_1", enc: "c2VhbGVk"
    )
    pushed.progress = "Approving…"
    let local = AgentActivityAttributes.ContentState(status: .blocked, statusSince: since, title: "claude", workspace: "api", approvals: 1)
    #expect(local.keepingApproval(of: pushed) == pushed)
    let moved = AgentActivityAttributes.ContentState(status: .working, statusSince: since, title: "claude", workspace: "api", approvals: 0)
    #expect(moved.keepingApproval(of: pushed) == moved)
    #expect(local.keepingApproval(of: moved) == local)
}

@Test func activitiesFromAnOlderBuildAreRestarted() throws {
    let old = #"{"machineId":"m1","terminalId":"t1","machineLabel":"Mac"}"#
    let attributes = try JSONDecoder().decode(AgentActivityAttributes.self, from: Data(old.utf8))
    #expect(attributes.nodeId == nil)
    #expect(attributes.isOutdated)
    #expect(attributes.title == nil)
    let state = AgentActivityAttributes.ContentState(status: .idle, statusSince: .now, title: "claude", workspace: nil, approvals: 0)
    #expect(attributes.displayTitle(state) == "claude")
    let current = AgentActivityAttributes(machineId: "m1", terminalId: "t1", machineLabel: "Mac", nodeId: "nMAC", title: "fix the build")
    #expect(!current.isOutdated)
    let decoded = try JSONDecoder().decode(AgentActivityAttributes.self, from: JSONEncoder().encode(current))
    #expect(decoded.nodeId == "nMAC")
    #expect(decoded.displayTitle(state) == "fix the build")
}

@MainActor
@Test func activityButtonsNeedUnlockAndDecideThroughTheNotificationPath() async throws {
    #expect(DecideApprovalIntent.authenticationPolicy == .requiresAuthentication)
    #expect(!DecideApprovalIntent.openAppWhenRun)
    var calls: [String] = []
    let previous = DecideApprovalIntent.decide
    DecideApprovalIntent.decide = { link, decision in calls.append("\(link.nodeId) \(link.approvalId) \(decision)") }
    defer { DecideApprovalIntent.decide = previous }

    _ = try await DecideApprovalIntent(nodeId: "nMAC123", approvalId: "ap_1", decision: .approve).perform()
    _ = try await DecideApprovalIntent(nodeId: "nMAC123", approvalId: "ap_2", decision: .deny).perform()
    #expect(calls == ["nMAC123 ap_1 approve", "nMAC123 ap_2 deny"])

    for (nodeId, approvalId) in [
        ("", "ap_1"), ("nMAC", ""), ("nMAC", "../ap"), ("n MAC", "ap_1"), ("nMAC", "ap_é"),
        (String(repeating: "a", count: 65), "ap_1"), ("nMAC", "w6:p1"),
    ] {
        await #expect(throws: DecideApprovalError.invalidIds) {
            _ = try await DecideApprovalIntent(nodeId: nodeId, approvalId: approvalId, decision: .approve).perform()
        }
    }
    #expect(calls.count == 2)
}

@Test func contentStateWithoutWorkspace() throws {
    let json = #"{"status":"idle","statusSince":812721600,"title":"agent","workspace":null,"approvals":0}"#
    let state = try JSONDecoder().decode(AgentActivityAttributes.ContentState.self, from: Data(json.utf8))
    #expect(state.workspace == nil)
    let absent = #"{"status":"idle","statusSince":812721600,"title":"agent","approvals":0}"#
    #expect(try JSONDecoder().decode(AgentActivityAttributes.ContentState.self, from: Data(absent.utf8)) == state)
}

private func summary(name: String? = nil, kind: String? = "claude", title: String? = "rm -rf secrets", status: AgentState = .blocked) -> AgentSummary {
    AgentSummary(
        terminalId: "term_1", workspaceId: "w1", kind: kind, name: name, title: title,
        status: status, statusSinceMs: 1_791_028_800_999, cwd: nil, lastLine: nil
    )
}

@Test func contentStateFromAgentNeverUsesTheTerminalTitle() {
    let state = AgentActivityAttributes.ContentState(agent: summary(), workspace: "api", approvals: 1)
    #expect(state.title == "claude")
    #expect(state.kind == "claude")
    #expect(AgentActivityAttributes.ContentState(agent: summary(kind: "gemini"), workspace: nil, approvals: 0).kind == nil)
    #expect(state.status == .blocked)
    #expect(state.statusSince == Date(timeIntervalSince1970: 1_791_028_800))
    #expect(AgentActivityAttributes.ContentState(agent: summary(name: " api-fixer "), workspace: nil, approvals: 0).title == "api-fixer")
    #expect(AgentActivityAttributes.ContentState(agent: summary(name: " ", kind: nil), workspace: nil, approvals: 0).title == "agent")
    #expect(summary(name: String(repeating: "x", count: 80)).alertTitle.count == 64)
    #expect(summary().activityTitle == "rm -rf secrets")
    #expect(summary(title: String(repeating: "x", count: 80)).activityTitle.count == 64)
    #expect(summary(title: " ").activityTitle == "claude")
    #expect(summary(title: "e" + String(repeating: "\u{301}", count: 2000)).activityTitle.utf8.count <= 256)
}

@Test func agentLinkRoundTrips() throws {
    let link = try #require(AgentLink(machineId: "0123abcd", terminalId: "w6:p1"))
    #expect(link.url.absoluteString == "collie://agent?m=0123abcd&t=w6:p1")
    #expect(AgentLink(url: link.url) == link)
    #expect(AgentLink(url: try #require(URL(string: "collie://agent?t=term_1&m=m1")))?.terminalId == "term_1")
}

@Test(arguments: [
    "collie://agent?m=m1",
    "collie://agent?t=term_1",
    "collie://pair?m=m1&t=term_1",
    "https://agent?m=m1&t=term_1",
    "collie://agent/x?m=m1&t=term_1",
    "collie://agent?m=m1&t=term_1&x=1",
    "collie://agent?m=m1&t=term_1&t=term_2",
    "collie://agent?m=m%201&t=term_1",
    "collie://agent?m=m.1&t=term_1",
    "collie://agent?m=m1&t=term%2F1",
    "collie://agent?m=&t=term_1",
    "collie://agent?m=m1&t=\(String(repeating: "a", count: 65))",
    "collie://agent?m=m1&t=%C3%A9",
])
func agentLinkRejects(_ string: String) throws {
    #expect(AgentLink(url: try #require(URL(string: string))) == nil)
}

@Test func agentLinkToAnUnpairedMacIsIgnored() throws {
    let machines = [Machine(id: "m1", label: "Mac", host: "mac.example.ts.net", port: 8457, nodeId: "nMAC", kind: .mac, key: "")]
    let url = try #require(URL(string: "collie://agent?m=m1&t=term_1"))
    #expect(AppModel.route(for: url, machines: machines) == AgentRoute(machineId: "m1", terminalId: "term_1"))
    #expect(AppModel.route(for: try #require(URL(string: "collie://agent?m=m2&t=term_1")), machines: machines) == nil)
}

private func followFile() -> URL {
    URL.temporaryDirectory.appending(path: "follows-\(UUID().uuidString).json")
}

@Test func followListPersists() {
    let file = followFile()
    defer { try? FileManager.default.removeItem(at: file) }
    #expect(FollowList.load(from: file).agents.isEmpty)
    var list = FollowList()
    var added: Bool
    added = list.add(AgentRoute(machineId: "m1", terminalId: "t1"))
    #expect(added)
    added = list.add(AgentRoute(machineId: "m1", terminalId: "t2"))
    #expect(added)
    list.save(to: file)
    #expect(FollowList.load(from: file) == list)
    list.remove(AgentRoute(machineId: "m1", terminalId: "t1"))
    list.save(to: file)
    #expect(FollowList.load(from: file).agents == [AgentRoute(machineId: "m1", terminalId: "t2")])
}

@Test func followListCapsAtFive() {
    var list = FollowList()
    var added: Bool
    for i in 1...5 {
        added = list.add(AgentRoute(machineId: "m1", terminalId: "t\(i)"))
        #expect(added)
    }
    #expect(list.isFull)
    added = list.add(AgentRoute(machineId: "m1", terminalId: "t3"))
    #expect(added)
    added = list.add(AgentRoute(machineId: "m1", terminalId: "t6"))
    #expect(!added)
    #expect(list.agents.count == 5)
}

@MainActor
@Test func sixthFollowExplainsTheLimit() {
    let file = followFile()
    defer { try? FileManager.default.removeItem(at: file) }
    var list = FollowList()
    for i in 1...5 { list.add(AgentRoute(machineId: "m1", terminalId: "t\(i)")) }
    list.save(to: file)
    let follows = FollowModel(core: nil, approvals: nil, file: file)
    #expect(follows.isFollowing(AgentRoute(machineId: "m1", terminalId: "t5")))
    follows.follow(AgentRoute(machineId: "m1", terminalId: "t6"))
    #expect(follows.notice?.contains("up to 5 agents") == true)
    #expect(!follows.isFollowing(AgentRoute(machineId: "m1", terminalId: "t6")))
    #expect(FollowList.load(from: file).agents.count == 5)
}

@MainActor
@Test func followsAreOffByDefault() {
    let follows = FollowModel(core: nil, approvals: nil, file: followFile())
    #expect(follows.list.agents.isEmpty)
    #expect(!follows.isFollowing(AgentRoute(machineId: "m1", terminalId: "t1")))
}

extension FakeCore: ActivityCore {
    func machines() -> [Machine] { state.withLock { $0.machines } }
    func cachedFlock(machineId: String) -> MachineFlock? { state.withLock { $0.cachedFlock } }
    func reconnect(machineId: String) throws {}
    func registerActivityToken(machineId: String, activityId: String, terminalId: String, tokenHex: String) throws {}
    func endActivity(machineId: String, activityId: String) throws {
        state.withLock { $0.endedActivities.append(activityId) }
    }
}

@Test func followListRemembersActivityIdsOnly() throws {
    let file = followFile()
    defer { try? FileManager.default.removeItem(at: file) }
    var list = FollowList()
    let activity = RegisteredActivity(machineId: "m1", activityId: "A1")
    var changed: Bool
    changed = list.remember(activity)
    #expect(changed)
    changed = list.remember(activity)
    #expect(!changed)
    list.save(to: file)
    #expect(FollowList.load(from: file).registered == [activity])
    let json = try #require(try JSONSerialization.jsonObject(with: Data(contentsOf: file)) as? [String: Any])
    #expect(Set(json.keys) == ["agents", "registered"])
    changed = list.forget(activityId: "A1")
    #expect(changed)
    changed = list.forget(activityId: "A1")
    #expect(!changed)
    #expect(list.registered.isEmpty)
}

private func listWithDeadActivity(file: URL, following: Bool) {
    var list = FollowList()
    if following { list.add(AgentRoute(machineId: "m1", terminalId: "t1")) }
    _ = list.remember(RegisteredActivity(machineId: "m1", activityId: "A1"))
    list.save(to: file)
}

@MainActor
@Test func foregroundEndsActivitiesIOSEndedWhileTheAppWasNotRunning() {
    let file = followFile()
    defer { try? FileManager.default.removeItem(at: file) }
    listWithDeadActivity(file: file, following: false)
    let core = FakeCore()
    FollowModel(core: core, approvals: nil, file: file).foreground()
    #expect(core.snapshot.endedActivities == ["A1"])
    #expect(FollowList.load(from: file).registered.isEmpty)
}

@MainActor
@Test func unfollowEndsAnActivityIOSAlreadyEnded() {
    let file = followFile()
    defer { try? FileManager.default.removeItem(at: file) }
    listWithDeadActivity(file: file, following: true)
    let core = FakeCore()
    FollowModel(core: core, approvals: nil, file: file).unfollow(AgentRoute(machineId: "m1", terminalId: "t1"))
    #expect(core.snapshot.endedActivities == ["A1"])
    let list = FollowList.load(from: file)
    #expect(list.registered.isEmpty)
    #expect(list.agents.isEmpty)
}
