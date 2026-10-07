import CryptoKit
import Foundation
import Testing

@testable import Collie

private func write(_ json: String, as name: String) throws -> URL {
    let dir = FileManager.default.temporaryDirectory.appending(path: UUID().uuidString, directoryHint: .isDirectory)
    try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
    let file = dir.appending(path: name)
    try Data(json.utf8).write(to: file)
    return file
}

private func object(_ data: Data) throws -> NSDictionary {
    try #require(try JSONSerialization.jsonObject(with: data) as? NSDictionary)
}

// Files as earlier builds wrote them: each must keep loading, and save the same JSON.

@Test func prefsFileLoadsAndSavesTheSameJSON() throws {
    let json = #"""
        {"wrapLines":false,"keepKeyboard":true,"gestures":{"doubleTap":"escape","tripleTap":"paste","pinchResizesText":false,"swipeSwitchesAgents":true},"fontSize":13,"dictationLanguage":"fr-CA","agentsLayout":"inbox","historyLines":500,"watchDecisions":true}
        """#
    let file = try write(json, as: "prefs.json")
    defer { try? FileManager.default.removeItem(at: file.deletingLastPathComponent()) }
    let prefs = DevicePrefs.load(from: file)
    #expect(
        prefs
            == DevicePrefs(
                wrapLines: false, keepKeyboard: true,
                gestures: TerminalGestures(doubleTap: .escape, tripleTap: .paste, pinchResizesText: false, swipeSwitchesAgents: true),
                fontSize: 13, dictationLanguage: .french, agentsLayout: .inbox, historyLines: 500, watchDecisions: true
            )
    )
    prefs.save(to: file)
    #expect(try object(Data(contentsOf: file)) == object(Data(json.utf8)))
}

@Test func draftsFileLoadsAndSavesTheSameJSON() throws {
    let json = #"""
        {"drafts":[{"machineId":"m1","terminalId":"term_1"},{"text":"fix it","attachments":[{"path":"/tmp/a/shot.png","name":"shot.png","uploaded":800000000}],"shell":false}]}
        """#
    let file = try write(json, as: "drafts.json")
    defer { try? FileManager.default.removeItem(at: file.deletingLastPathComponent()) }
    let drafts = AgentDrafts.load(from: file)
    let draft = try #require(drafts.drafts[AgentRoute(machineId: "m1", terminalId: "term_1")])
    #expect(draft.text == "fix it")
    #expect(!draft.shell)
    #expect(draft.attachments.map(\.path) == ["/tmp/a/shot.png"])
    #expect(draft.attachments.first?.uploaded == Date(timeIntervalSinceReferenceDate: 800_000_000))
    drafts.save(to: file)
    #expect(try object(Data(contentsOf: file)) == object(Data(json.utf8)))
}

@Test func followsFileLoadsAndSavesTheSameJSON() throws {
    let json = #"""
        {"agents":[{"machineId":"m1","terminalId":"term_1"}],"registered":[{"machineId":"m1","activityId":"act-1"}]}
        """#
    let file = try write(json, as: "follows.json")
    defer { try? FileManager.default.removeItem(at: file.deletingLastPathComponent()) }
    let list = FollowList.load(from: file)
    #expect(list.agents == [AgentRoute(machineId: "m1", terminalId: "term_1")])
    #expect(list.registered == [RegisteredActivity(machineId: "m1", activityId: "act-1")])
    list.save(to: file)
    #expect(try object(Data(contentsOf: file)) == object(Data(json.utf8)))
}

@Test func unreadableStateFilesLoadAsTheDefault() throws {
    let file = try write("{", as: "prefs.json")
    defer { try? FileManager.default.removeItem(at: file.deletingLastPathComponent()) }
    #expect(DevicePrefs.load(from: file) == DevicePrefs())
    #expect(AgentDrafts.load(from: file).drafts.isEmpty)
    #expect(FollowList.load(from: file) == FollowList())
}

// The simulator reports no data protection class, so only the mode and the backup exclusion are checked.

@Test func stateDirectoryIsPrivateAndNotBackedUp() throws {
    let dir = try StateDirectory.prepare()
    let attributes = try FileManager.default.attributesOfItem(atPath: dir.path)
    #expect((attributes[.posixPermissions] as? NSNumber)?.intValue == 0o700)
    #expect(try dir.resourceValues(forKeys: [.isExcludedFromBackupKey]).isExcludedFromBackup == true)
}

/// Needs the App Group entitlement, which an unsigned simulator build lacks.
@Test(.enabled(if: AppGroup.container != nil))
func notificationKeyMirrorIsPrivateAndNotBackedUp() throws {
    let nodeId = "nTEST\(UUID().uuidString.prefix(8))"
    let url = try #require(NotificationKey.Mirror.url(nodeId: nodeId))
    defer { NotificationKey.Mirror.delete(nodeId: nodeId) }
    try NotificationKey.Mirror.write(vectorKey, nodeId: nodeId)
    let dir = url.deletingLastPathComponent()
    let file = try FileManager.default.attributesOfItem(atPath: url.path)
    #expect((try FileManager.default.attributesOfItem(atPath: dir.path)[.posixPermissions] as? NSNumber)?.intValue == 0o700)
    #expect((file[.posixPermissions] as? NSNumber)?.intValue == 0o600)
    #expect(try dir.resourceValues(forKeys: [.isExcludedFromBackupKey]).isExcludedFromBackup == true)
    #expect(try url.resourceValues(forKeys: [.isExcludedFromBackupKey]).isExcludedFromBackup == true)
    #expect(NotificationKey.Mirror.read(nodeId: nodeId).map { $0.withUnsafeBytes { Data($0) } } == vectorKey.withUnsafeBytes { Data($0) })
    NotificationKey.Mirror.delete(nodeId: nodeId)
    #expect(!FileManager.default.fileExists(atPath: url.path))
}
