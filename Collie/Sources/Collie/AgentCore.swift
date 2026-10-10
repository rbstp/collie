import CollieCore
import Foundation

/// The slice of CollieCore the agent screens use, so view models can run against a fake.
protocol AgentCore: AnyObject, Sendable {
    func agentView(machineId: String, terminalId: String, afterRevision: UInt64) -> AgentView?
    func watchAgent(machineId: String, terminalId: String?, lines: UInt16) async throws
    func agentRead(machineId: String, terminalId: String, source: TerminalSource, lines: UInt16?) async throws -> TerminalSnapshot
    func agentDraft(machineId: String, terminalId: String) async throws -> String?
    func prompt(machineId: String, terminalId: String, text: String, expectedDraft: String?) async throws
    func sendKeys(machineId: String, terminalId: String, keys: [AgentKey]) async throws
    func typeText(machineId: String, terminalId: String, text: String) async throws
    func focus(machineId: String, terminalId: String) async throws
    func scrollBottom(machineId: String, terminalId: String) async throws
    func answerNotice(machineId: String, terminalId: String, digit: UInt8, label: String) async throws
    func slashDraft(machineId: String, terminalId: String, command: String, expectedDraft: String?) async throws
    func star(machineId: String, terminalId: String, starred: Bool) async throws
    func closeWorkspace(machineId: String, workspaceId: String, confirm: Bool) async throws
    func closePane(machineId: String, terminalId: String, confirm: Bool) async throws
    func archiveTask(machineId: String, terminalId: String, confirm: Bool) async throws -> String
    func taskOptions(machineId: String) async throws -> TaskOptions
    func taskFolders(machineId: String, path: String) async throws -> TaskFolders
    func taskNew(machineId: String, cwd: String, agent: String, prompt: String, label: String?, newFolder: String?) async throws -> TaskStarted
    func taskWorktrees(machineId: String, cwd: String) async throws -> WorktreeListing
    func taskWorktreeCreate(machineId: String, cwd: String, branch: String, agent: String, prompt: String, label: String?) async throws -> TaskStarted
    func taskWorktreeOpen(machineId: String, cwd: String, path: String, agent: String, prompt: String, label: String?) async throws -> TaskStarted
    func flock(machineId: String) async throws -> MachineFlock
    func uploadAttachment(machineId: String, name: String, data: Data, progress: any UploadProgress) async throws -> String
    func maxAttachmentBytes() -> UInt64
    func cancelUploads(machineId: String)
    func terminalChallenge(machineId: String, terminalId: String) async throws -> Data
    func terminalGrant(machineId: String, terminalId: String, signature: Data) async throws
    func watchTerminal(machineId: String, terminalId: String, lines: UInt16) async throws
    func terminalRun(machineId: String, terminalId: String, text: String) async throws
    func terminalSendKeys(machineId: String, terminalId: String, keys: [AgentKey]) async throws
}

extension CollieCore: AgentCore {}

struct AgentRoute: Hashable, Codable {
    let machineId: String
    let terminalId: String
}
