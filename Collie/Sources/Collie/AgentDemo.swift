#if DEBUG
import CollieCore
import Foundation
import SwiftUI

/// `--terminal-demo <file>`: the agent screen over the first "text" string of a herdr JSON
/// response (or the raw file), with no Mac. Debug builds only.
struct AgentDemo: View {
    let core: DemoAgentCore

    init?(arguments: [String]) {
        guard let flag = arguments.firstIndex(of: "--terminal-demo"), flag + 1 < arguments.count,
            let data = FileManager.default.contents(atPath: arguments[flag + 1])
        else { return nil }
        let json = try? JSONSerialization.jsonObject(with: data)
        core = DemoAgentCore(snapshot: json.flatMap(Self.firstText) ?? String(decoding: data, as: UTF8.self))
    }

    var body: some View {
        NavigationStack {
            AgentScreen(core: core, route: DemoAgentCore.route)
        }
    }

    private static func firstText(_ value: Any) -> String? {
        if let object = value as? [String: Any] {
            if let text = object["text"] as? String { return text }
            return object.keys.sorted().lazy.compactMap { object[$0].flatMap(firstText) }.first
        }
        if let array = value as? [Any] {
            return array.lazy.compactMap(firstText).first
        }
        return nil
    }
}

final class DemoAgentCore: AgentCore {
    static let route = AgentRoute(machineId: "demo", terminalId: "term_demo")

    let snapshot: String
    private let since = UInt64(Date.now.timeIntervalSince1970 * 1000) - 135_000

    init(snapshot: String) {
        self.snapshot = snapshot
    }

    func agentView(machineId: String, terminalId: String, afterRevision: UInt64) -> AgentView? {
        AgentView(
            link: .connected, lastError: nil,
            agent: AgentSummary(
                terminalId: terminalId, workspaceId: "ws_demo", kind: "claude", name: nil, title: "fix the build",
                status: .working, statusSinceMs: since, cwd: "/Users/demo/project", lastLine: nil
            ),
            output: afterRevision < 1 ? read(terminalId) : nil,
            outputRevision: 1
        )
    }

    func watchAgent(machineId: String, terminalId: String?) async throws {}

    func agentRead(machineId: String, terminalId: String, source: TerminalSource) async throws -> TerminalSnapshot {
        read(terminalId)
    }

    func prompt(machineId: String, terminalId: String, text: String) async throws {
        try await Task.sleep(for: .milliseconds(500))
    }

    func sendKeys(machineId: String, terminalId: String, keys: [AgentKey]) async throws {}

    func focus(machineId: String, terminalId: String) async throws {}

    func closeWorkspace(machineId: String, workspaceId: String, confirm: Bool) async throws {
        throw CoreError.NotImplemented
    }

    func closePane(machineId: String, terminalId: String, confirm: Bool) async throws {
        throw CoreError.NotImplemented
    }

    func taskOptions(machineId: String) async throws -> TaskOptions {
        throw CoreError.NotImplemented
    }

    func taskNew(machineId: String, cwd: String, agent: String, prompt: String, label: String?) async throws -> TaskStarted {
        throw CoreError.NotImplemented
    }

    func flock(machineId: String) async throws -> MachineFlock {
        throw CoreError.MachineNotFound
    }

    private func read(_ terminalId: String) -> TerminalSnapshot {
        TerminalSnapshot(terminalId: terminalId, source: .recent, ansi: snapshot, truncated: false)
    }
}
#endif
