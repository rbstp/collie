import CollieCore
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
    #expect(state.workspace == "api")
    #expect(state.approvals == 1)
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
    #expect(state.status == .blocked)
    #expect(state.statusSince == Date(timeIntervalSince1970: 1_791_028_800))
    #expect(AgentActivityAttributes.ContentState(agent: summary(name: " api-fixer "), workspace: nil, approvals: 0).title == "api-fixer")
    #expect(AgentActivityAttributes.ContentState(agent: summary(name: " ", kind: nil), workspace: nil, approvals: 0).title == "agent")
    #expect(summary(name: String(repeating: "x", count: 80)).alertTitle.count == 64)
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
    let machines = [Machine(id: "m1", label: "Mac", host: "mac.example.ts.net", port: 8457, nodeId: "nMAC")]
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
    added = list.add(FollowedAgent(AgentRoute(machineId: "m1", terminalId: "t1")))
    #expect(added)
    added = list.add(FollowedAgent(AgentRoute(machineId: "m1", terminalId: "t2")))
    #expect(added)
    list.save(to: file)
    #expect(FollowList.load(from: file) == list)
    list.remove(FollowedAgent(AgentRoute(machineId: "m1", terminalId: "t1")))
    list.save(to: file)
    #expect(FollowList.load(from: file).agents == [FollowedAgent(AgentRoute(machineId: "m1", terminalId: "t2"))])
}

@Test func followListCapsAtFive() {
    var list = FollowList()
    var added: Bool
    for i in 1...5 {
        added = list.add(FollowedAgent(AgentRoute(machineId: "m1", terminalId: "t\(i)")))
        #expect(added)
    }
    #expect(list.isFull)
    added = list.add(FollowedAgent(AgentRoute(machineId: "m1", terminalId: "t3")))
    #expect(added)
    added = list.add(FollowedAgent(AgentRoute(machineId: "m1", terminalId: "t6")))
    #expect(!added)
    #expect(list.agents.count == 5)
}

@MainActor
@Test func sixthFollowExplainsTheLimit() {
    let file = followFile()
    defer { try? FileManager.default.removeItem(at: file) }
    var list = FollowList()
    for i in 1...5 { list.add(FollowedAgent(AgentRoute(machineId: "m1", terminalId: "t\(i)"))) }
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
    func machines() -> [Machine] { [] }
    func cachedFlock(machineId: String) -> MachineFlock? { nil }
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
    if following { list.add(FollowedAgent(AgentRoute(machineId: "m1", terminalId: "t1"))) }
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
