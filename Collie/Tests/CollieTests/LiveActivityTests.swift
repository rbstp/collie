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

private func displayVectors() throws -> [[String: Any]] {
    let url = URL(filePath: #filePath).deletingLastPathComponent().appending(path: "../../../docs/protocol/push-display-vectors.json")
    let json = try #require(try JSONSerialization.jsonObject(with: Data(contentsOf: url)) as? [String: Any])
    return try #require(json["vectors"] as? [[String: Any]])
}

@Test func encryptedStateOpensOnlyForItsMachineActivityAndKey() throws {
    let vector = try #require(try displayVectors().first { $0["kind"] as? Int == 2 })
    let enc = try #require(vector["enc"] as? String)
    let json = try JSONSerialization.data(withJSONObject: ["v": 1, "enc": enc])
    let stored = try JSONDecoder().decode(AgentActivityAttributes.ContentState.self, from: json)
    #expect(stored.status == .unknown)
    #expect(stored.pendingApproval == nil)
    let state = stored.opened(nodeId: "nMAC", scope: "binding-test", key: vectorKey)
    #expect(state.status == .blocked)
    #expect(state.statusSince == Date(timeIntervalSince1970: 1_791_028_800))
    #expect(state.title == "api-fixer")
    #expect(state.kind == "claude")
    #expect(state.workspace == "api")
    #expect(state.approvals == 1)
    #expect(state.pendingApproval == "apr_test")
    #expect(state.command(key: vectorKey) == "Bash: echo hi")
    for neutral in [stored.opened(nodeId: "nOther", scope: "binding-test", key: vectorKey),
        stored.opened(nodeId: "nMAC", scope: "another-activity", key: vectorKey),
        stored.opened(nodeId: "nMAC", scope: "binding-test", key: SymmetricKey(size: .bits256))] {
        #expect(neutral.status == .unknown)
        #expect(neutral.title == "Collie")
        #expect(neutral.pendingApproval == nil)
        #expect(neutral.command(key: vectorKey) == nil)
    }
    let encoded = try #require(try JSONSerialization.jsonObject(with: JSONEncoder().encode(state)) as? [String: Any])
    #expect(Set(encoded.keys) == ["v", "enc"])
    #expect(encoded["enc"] as? String == enc)
}

@Test func ciphertextChangesRemainDistinctToActivityKit() throws {
    let first = try JSONDecoder().decode(AgentActivityAttributes.ContentState.self, from: Data(#"{"v":1,"enc":"first"}"#.utf8))
    let second = try JSONDecoder().decode(AgentActivityAttributes.ContentState.self, from: Data(#"{"v":1,"enc":"second"}"#.utf8))
    #expect(first != second)
    #expect(first.sameDisplay(as: second))
}

@Test func privateStateNeverEncodesWithoutAnAuthenticatedEnvelope() throws {
    let state = AgentActivityAttributes.ContentState(status: .blocked, statusSince: .now, title: "Private title", workspace: "Private workspace", approvals: 1, approvalId: "apr_private")
    let encoded = try JSONEncoder().encode(state)
    let text = try #require(String(data: encoded, encoding: .utf8))
    #expect(!text.contains("Private"))
    #expect(!text.contains("apr_private"))
    let stored = try JSONDecoder().decode(AgentActivityAttributes.ContentState.self, from: encoded)
    #expect(stored.pendingApproval == nil)
    #expect(stored.status == .unknown)
}

@Test func kindsMatchCollied() throws {
    #expect(try fixture()["kinds"] as? [String] == AgentActivityAttributes.ContentState.kinds)
}

@Test func everyStatusDecodesAndUnknownOnesFallBack() throws {
    let statuses = try #require(try fixture()["statuses"] as? [String])
    for status in statuses {
        let decoded = try JSONDecoder().decode(AgentActivityStatus.self, from: JSONEncoder().encode(status))
        #expect(decoded.rawValue == status)
    }
    #expect(try JSONDecoder().decode(AgentActivityStatus.self, from: Data(#""sleeping""#.utf8)) == .unknown)
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

@Test func activityAttributesCarryNoPrivateLabelsEvenWithoutAKey() throws {
    let current = AgentActivityAttributes(machineId: "m1", terminalId: "t1", machineLabel: "Private machine", nodeId: "nNoKey", title: "Private title")
    let encoded = try JSONEncoder().encode(current)
    let object = try #require(try JSONSerialization.jsonObject(with: encoded) as? [String: Any])
    #expect(Set(object.keys) == ["machineId", "terminalId", "nodeId", "bindingId", "enc"])
    let text = try #require(String(data: encoded, encoding: .utf8))
    #expect(!text.contains("Private"))
    let decoded = try JSONDecoder().decode(AgentActivityAttributes.self, from: encoded)
    #expect(decoded.nodeId == "nNoKey")
    #expect(decoded.machineLabel == "")
    #expect(decoded.title == nil)
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
    func registerActivityToken(machineId: String, activityId: String, terminalId: String, tokenHex: String, bindingId: String) throws {}
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
