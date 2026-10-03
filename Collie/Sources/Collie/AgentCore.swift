import CollieCore
import Foundation

/// The slice of CollieCore the agent screens use, so view models can run against a fake.
protocol AgentCore: AnyObject, Sendable {
    func agentView(machineId: String, terminalId: String, afterRevision: UInt64) -> AgentView?
    func watchAgent(machineId: String, terminalId: String?) async throws
    func agentRead(machineId: String, terminalId: String, source: TerminalSource) async throws -> TerminalSnapshot
    func prompt(machineId: String, terminalId: String, text: String) async throws
    func sendKeys(machineId: String, terminalId: String, keys: [AgentKey]) async throws
    func focus(machineId: String, terminalId: String) async throws
    func closeWorkspace(machineId: String, workspaceId: String, confirm: Bool) async throws
    func closePane(machineId: String, terminalId: String, confirm: Bool) async throws
    func taskOptions(machineId: String) async throws -> TaskOptions
    func taskNew(machineId: String, cwd: String, agent: String, prompt: String, label: String?) async throws -> TaskStarted
    func flock(machineId: String) async throws -> MachineFlock
    func uploadAttachment(machineId: String, name: String, data: Data, progress: any UploadProgress) async throws -> String
    func maxAttachmentBytes() -> UInt64
    func cancelUploads(machineId: String)
}

extension CollieCore: AgentCore {}

struct AgentRoute: Hashable {
    let machineId: String
    let terminalId: String
}
