import CollieCore
import Foundation
import Testing
import UIKit

@testable import Collie

private func draftsFile() throws -> (file: URL, dir: URL) {
    let dir = FileManager.default.temporaryDirectory.appending(path: UUID().uuidString, directoryHint: .isDirectory)
    try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
    return (dir.appending(path: "drafts.json"), dir)
}

@MainActor
private func screen(_ core: FakeCore, _ file: URL, _ terminalId: String = "term_1", on machineId: String = "m1") -> AgentModel {
    AgentModel(core: core, route: AgentRoute(machineId: machineId, terminalId: terminalId), prefsFile: nil, draftsFile: file)
}

private func route(_ terminalId: String, on machineId: String = "m1") -> AgentRoute {
    AgentRoute(machineId: machineId, terminalId: terminalId)
}

private func entry(_ machineId: String, loaded: Bool = true, agents: [String] = [], terminals: [String] = []) -> MachineFlockEntry {
    let machine = Machine(id: machineId, label: "Mac", host: "mac.ts.net", port: 8457, nodeId: "n1", kind: .mac, key: "")
    let flock = MachineFlock(
        machine: machine, link: .connected, lastError: nil,
        details: loaded ? MachineDetails(name: "Mac", nodeId: "n1", herdrSession: "default") : nil, workspaces: [],
        agents: agents.map {
            AgentSummary(terminalId: $0, workspaceId: "w1", kind: "claude", name: nil, title: nil, status: .idle, statusSinceMs: 0, cwd: nil, lastLine: nil)
        },
        approvalsCount: 0,
        terminals: terminals.map { TerminalSummary(terminalId: $0, workspaceId: "w1", workspaceLabel: nil, label: nil, cwd: nil, locked: true) },
        terminalsEnabled: true
    )
    return MachineFlockEntry(machine: machine, flock: flock)
}

@MainActor
@Test func aDraftIsKeptPerAgentAndRestoredWhenItsScreenOpensAgain() async throws {
    let (file, dir) = try draftsFile()
    defer { try? FileManager.default.removeItem(at: dir) }
    let core = FakeCore()

    let first = screen(core, file)
    first.draft = "fix the build\n"
    await first.attach(name: "notes.txt") { _ in Data([1]) }?.value
    first.saveDraft()
    let second = screen(core, file, "term_2")
    second.draft = "and the tests"
    second.saveDraft()
    let blank = screen(core, file, "term_3")
    blank.draft = " \n"
    blank.saveDraft()

    let reopened = screen(core, file)
    #expect(reopened.draft == "fix the build\n")
    #expect(reopened.attachments.map(\.path) == [core.snapshot.uploadPath])
    #expect(reopened.attachments.map(\.name) == ["notes.txt"])
    #expect(screen(core, file, "term_2").draft == "and the tests")
    #expect(screen(core, file, "term_2", on: "m2").draft.isEmpty)
    #expect(AgentDrafts.load(from: file).drafts.keys.sorted { $0.terminalId < $1.terminalId } == [route("term_1"), route("term_2")])

    reopened.draft = ""
    reopened.remove(reopened.attachments[0])
    reopened.saveDraft()
    #expect(AgentDrafts.load(from: file).drafts[route("term_1")] == nil)
}

@MainActor
@Test func aSentDraftIsDroppedAndAFailedOneKept() async throws {
    let (file, dir) = try draftsFile()
    defer { try? FileManager.default.removeItem(at: dir) }
    let core = FakeCore()

    let model = screen(core, file)
    model.draft = "continue"
    model.saveDraft()
    core.set(error: .AgentBlocked)
    await model.sendPrompt()
    #expect(screen(core, file).draft == "continue")

    core.set(error: nil)
    await model.sendPrompt()
    #expect(core.snapshot.prompts == ["continue", "continue"])
    #expect(AgentDrafts.load(from: file).drafts.isEmpty)
    #expect(screen(core, file).draft.isEmpty)
}

@MainActor
@Test func restoredAttachmentsKeepTheirThumbnailUntilTheMachineDeletesThem() throws {
    let (file, dir) = try draftsFile()
    defer { try? FileManager.default.removeItem(at: dir) }
    var fresh = AttachedFile(path: "/tmp/a/photo.jpg", name: "photo.jpg")
    fresh.thumbnail = UIGraphicsImageRenderer(size: CGSize(width: 4, height: 4)).image { context in
        UIColor.red.setFill()
        context.fill(CGRect(x: 0, y: 0, width: 4, height: 4))
    }
    var stale = AttachedFile(path: "/tmp/b/old.txt", name: "old.txt")
    stale.uploaded = .now - Attachment.keptOnMachine - 60
    AgentDrafts(drafts: [route("term_1"): .init(text: "", attachments: [stale, fresh])]).save(to: file)

    let model = screen(FakeCore(), file)
    #expect(model.attachments.map(\.name) == ["photo.jpg"])
    #expect(model.attachments.first?.kind == .image)
    #expect(model.attachments.first?.thumbnail != nil)
}

@MainActor
@Test func aDraftGoesOnceItsPaneOrMachineIsGone() throws {
    let (file, dir) = try draftsFile()
    defer { try? FileManager.default.removeItem(at: dir) }
    let draft = AgentDrafts.Draft(text: "x", attachments: [])
    AgentDrafts(drafts: [
        route("agent"): draft, route("shell"): draft, route("closed"): draft,
        route("offline", on: "m2"): draft, route("unpaired", on: "m3"): draft,
    ]).save(to: file)
    let routes = { Set(AgentDrafts.load(from: file).drafts.keys) }
    let all = routes()

    // Not yet loaded, or nothing listed yet: every draft stays.
    AgentDrafts.prune([], file: file)
    AgentDrafts.prune([entry("m1", loaded: false), entry("m2", loaded: false), entry("m3", loaded: false)], file: file)
    #expect(routes() == all)

    AgentDrafts.prune([entry("m1", agents: ["agent"], terminals: ["shell"]), entry("m2", loaded: false)], file: file)
    #expect(routes() == [route("agent"), route("shell"), route("offline", on: "m2")])
}

@MainActor
@Test func theMacDraftFillsTheFieldOnlyWhenThePhoneHasNone() async throws {
    let (file, dir) = try draftsFile()
    defer { try? FileManager.default.removeItem(at: dir) }
    let core = FakeCore()
    core.state.withLock {
        $0.kind = "claude"
        $0.macDraft = "from the mac"
    }

    let fromMac = screen(core, file)
    fromMac.poll()
    await fromMac.loadMacDraft()
    #expect(fromMac.draft == "from the mac")
    fromMac.saveDraft()
    #expect(AgentDrafts.load(from: file).drafts.isEmpty)

    let typed = screen(core, file)
    typed.draft = "from the phone"
    typed.saveDraft()
    let reopened = screen(core, file)
    reopened.poll()
    await reopened.loadMacDraft()
    #expect(reopened.draft == "from the phone")
    #expect(reopened.macDraft == nil)
}
