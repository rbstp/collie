#if DEBUG
import ActivityKit
import CollieCore
import CryptoKit
import Foundation
import SwiftUI
import UserNotifications

/// `--terminal-demo <file>`: the Agents list over fake agents, each agent screen showing the
/// first "text" string of a herdr JSON response (or the raw file), with no Mac. Debug builds only.
/// `--follow-demo <terminal id>` also follows that agent, which starts its Live Activity, and
/// gives a blocked one the approval fields collied would push.
struct AgentDemo: View {
    let core: DemoAgentCore
    private let followed: String?
    @State private var follows: FollowModel
    @State private var opening: AgentRoute?

    init?(arguments: [String]) {
        guard let flag = arguments.firstIndex(of: "--terminal-demo"), flag + 1 < arguments.count,
            let data = FileManager.default.contents(atPath: arguments[flag + 1])
        else { return nil }
        let json = try? JSONSerialization.jsonObject(with: data)
        core = DemoAgentCore(snapshot: json.flatMap(Self.firstText) ?? String(decoding: data, as: UTF8.self))
        _follows = State(initialValue: FollowModel(core: core, approvals: nil, file: nil))
        followed = arguments.firstIndex(of: "--follow-demo").flatMap { $0 + 1 < arguments.count ? arguments[$0 + 1] : nil }
    }

    var body: some View {
        TabView {
            Tab("Agents", systemImage: "square.grid.2x2") {
                FlockScreen(core: core, machines: [DemoAgentCore.machine], approvals: nil, follows: follows, opening: $opening)
            }
            Tab("Approvals", systemImage: "checkmark.shield") { Color.clear }
            Tab("Machines", systemImage: "desktopcomputer") { Color.clear }
            Tab("Settings", systemImage: "gearshape") { Color.clear }
        }
        .task {
            follows.foreground()
            if let followed {
                follows.follow(AgentRoute(machineId: DemoAgentCore.machine.id, terminalId: followed))
                await Self.pushDemoApproval()
            }
        }
        .onOpenURL { opening = AppModel.route(for: $0, machines: [DemoAgentCore.machine]) }
    }

    /// Sealed like collied's `enc`, with a key stored for the demo Mac as pairing would.
    private nonisolated static func pushDemoApproval() async {
        let approvalId = "ap_demo"
        let plaintext = #"{"v":1,"body":"Bash: Run the approval tests\ncargo test -p collied --test approvals -- --nocapture"}"#
        guard let key = try? NotificationKey.loadOrCreate(nodeId: DemoAgentCore.machine.nodeId),
            let sealed = try? ChaChaPoly.seal(Data(plaintext.utf8), using: key, authenticating: Data(approvalId.utf8))
        else { return }
        for activity in Activity<AgentActivityAttributes>.activities where activity.content.state.status == .blocked {
            var state = activity.content.state
            state.approvalId = approvalId
            state.enc = sealed.combined.base64EncodedString()
            await activity.update(FollowModel.activityContent(state))
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

final class DemoAgentCore: ActivityCore {
    static let machine = Machine(id: "demo", label: "MacBook Pro", host: "mac.example.ts.net", port: 8457, nodeId: "nDEMO", kind: .mac)

    let snapshot: String
    private let agents: [AgentSummary]

    init(snapshot: String) {
        self.snapshot = snapshot
        let now = UInt64(Date.now.timeIntervalSince1970 * 1000)
        agents = [
            AgentSummary(
                terminalId: "term_demo", workspaceId: "ws_collie", kind: "claude", name: nil, title: "fix the build",
                status: .working, statusSinceMs: now - 135_000, cwd: "/Users/demo/collie", lastLine: nil
            ),
            AgentSummary(
                terminalId: "term_2", workspaceId: "ws_collie", kind: "codex", name: nil, title: "add approval tests",
                status: .blocked, statusSinceMs: now - 42_000, cwd: "/Users/demo/collie", lastLine: nil
            ),
            AgentSummary(
                terminalId: "term_3", workspaceId: "ws_site", kind: "claude", name: nil, title: "update the docs",
                status: .idle, statusSinceMs: now - 900_000, cwd: "/Users/demo/website", lastLine: nil
            ),
        ]
    }

    func machines() -> [Machine] { [Self.machine] }

    func cachedFlock(machineId: String) -> MachineFlock? { demoFlock }

    func agentView(machineId: String, terminalId: String, afterRevision: UInt64) -> AgentView? {
        AgentView(
            link: .connected, lastError: nil,
            agent: agents.first { $0.terminalId == terminalId },
            output: afterRevision < 1 ? read(terminalId) : nil,
            outputRevision: 1
        )
    }

    func watchAgent(machineId: String, terminalId: String?) async throws {}

    func agentRead(machineId: String, terminalId: String, source: TerminalSource) async throws -> TerminalSnapshot {
        read(terminalId)
    }

    func agentDraft(machineId: String, terminalId: String) async throws -> String? { nil }

    func prompt(machineId: String, terminalId: String, text: String, expectedDraft: String?) async throws {
        try await Task.sleep(for: .milliseconds(500))
    }

    func sendKeys(machineId: String, terminalId: String, keys: [AgentKey]) async throws {}

    func typeText(machineId: String, terminalId: String, text: String) async throws {}

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

    func flock(machineId: String) async throws -> MachineFlock { demoFlock }

    func uploadAttachment(machineId: String, name: String, data: Data, progress: any UploadProgress) async throws -> String {
        let total = UInt64(data.count)
        for step in 1...10 {
            try await Task.sleep(for: .milliseconds(150))
            progress.onProgress(sent: total * UInt64(step) / 10, total: total)
        }
        return "/Users/demo/Library/Caches/dev.rbstp.collied/attachments/0123456789abcdef/\(name)"
    }

    func maxAttachmentBytes() -> UInt64 { 20 * 1024 * 1024 }

    func cancelUploads(machineId: String) {}

    func registerActivityToken(machineId: String, activityId: String, terminalId: String, tokenHex: String) throws {
        print("demo: activity \(activityId) token \(tokenHex.prefix(8))… for \(terminalId)")
    }

    func endActivity(machineId: String, activityId: String) throws {}

    private var demoFlock: MachineFlock {
        MachineFlock(
            machine: Self.machine, link: .connected, lastError: nil,
            details: MachineDetails(name: "MacBook Pro", nodeId: "nDEMO", herdrSession: "default"),
            workspaces: [
                WorkspaceSummary(workspaceId: "ws_collie", label: "collie", number: 1, status: .blocked, cwd: "/Users/demo/collie"),
                WorkspaceSummary(workspaceId: "ws_site", label: "website", number: 2, status: .idle, cwd: "/Users/demo/website"),
            ],
            agents: agents, approvalsCount: 1
        )
    }

    private func read(_ terminalId: String) -> TerminalSnapshot {
        TerminalSnapshot(terminalId: terminalId, source: .recent, ansi: snapshot, truncated: false)
    }
}

/// `--approvals-demo`: the Approvals tab over fake approvals, with no Mac. Debug builds only.
struct ApprovalsDemo: View {
    @State private var model = ApprovalsModel(core: DemoApprovalCore(), auth: DemoAuthenticator())
    @State private var tab = AppTab.approvals

    init?(arguments: [String]) {
        guard arguments.contains("--approvals-demo") else { return nil }
    }

    var body: some View {
        TabView(selection: $tab) {
            Tab("Agents", systemImage: "square.grid.2x2", value: AppTab.agents) { Color.clear }
            Tab("Approvals", systemImage: "checkmark.shield", value: AppTab.approvals) { ApprovalsScreen(model: model) }
                .badge(model.items.count)
            Tab("Machines", systemImage: "desktopcomputer", value: AppTab.machines) { Color.clear }
            Tab("Settings", systemImage: "gearshape", value: AppTab.settings) { Color.clear }
        }
        .task { _ = try? await UNUserNotificationCenter.current().requestAuthorization(options: [.alert, .sound, .badge]) }
    }
}

struct DemoAuthenticator: Authenticator {
    func authenticate(reason: String) async -> Bool { true }
}

final class DemoApprovalCore: ApprovalCore {
    private let machine = Machine(id: "demo", label: "MacBook Pro", host: "mac.example.ts.net", port: 8457, nodeId: "nDEMO", kind: .mac)
    private let pending: [PendingApproval]

    init() {
        let now = UInt64(Date.now.timeIntervalSince1970 * 1000)
        pending = [
            PendingApproval(
                approvalId: "ap_demo1", terminalId: "term_1", agentLabel: "fix the flaky test", workspaceLabel: "collie",
                snippet: """
                Bash command
                  Run the approval tests
                  cargo test -p collied --test approvals
                Do you want to proceed?
                > 1. Yes
                  2. Yes, and don't ask again for cargo test commands
                  3. No
                Esc to cancel · Tab to amend
                """,
                toolName: "Bash", toolSummary: "cargo test -p collied --test approvals",
                options: [.approve, .approveAlways, .deny],
                choices: [
                    ApprovalChoice(index: 0, label: "Yes", current: true),
                    ApprovalChoice(index: 1, label: "Yes, and don't ask again for cargo test commands", current: false),
                    ApprovalChoice(index: 2, label: "No", current: false),
                ],
                acceptsInput: false, hasTextField: false, supportsNote: true,
                createdAtMs: now - 45_000, expiresAtMs: now + 555_000
            ),
            PendingApproval(
                approvalId: "ap_demo2", terminalId: "term_2", agentLabel: "claude", workspaceLabel: "website",
                snippet: "Do you trust the files in this folder?\n> 1. Yes, proceed\n  2. No, exit",
                toolName: nil, toolSummary: nil,
                options: [.approve, .deny],
                choices: [
                    ApprovalChoice(index: 0, label: "Yes, proceed", current: true),
                    ApprovalChoice(index: 1, label: "No, exit", current: false),
                ],
                acceptsInput: false, hasTextField: false, supportsNote: false,
                createdAtMs: now - 10_000, expiresAtMs: now + 190_000
            ),
            PendingApproval(
                approvalId: "ap_demo3", terminalId: "term_3", agentLabel: "plan the migration", workspaceLabel: "collie",
                snippet: """
                Which database should the migration target?
                > 1. PostgreSQL 17
                  2. SQLite
                  3. Type something.
                """,
                toolName: nil, toolSummary: nil,
                options: [],
                choices: [
                    ApprovalChoice(index: 0, label: "PostgreSQL 17", current: true),
                    ApprovalChoice(index: 1, label: "SQLite", current: false),
                    ApprovalChoice(index: 2, label: "Type something.", current: false),
                ],
                acceptsInput: true, hasTextField: true, supportsNote: false,
                createdAtMs: now - 60_000, expiresAtMs: now + 540_000
            ),
            PendingApproval(
                approvalId: "ap_demo4", terminalId: "term_4", agentLabel: "add dark mode", workspaceLabel: "website",
                snippet: """
                Ready to code?
                Would you like to proceed?
                > 1. Yes, and use auto mode
                  2. Yes, manually approve edits
                  3. Tell Claude what to change
                """,
                toolName: nil, toolSummary: nil,
                options: [],
                choices: [
                    ApprovalChoice(index: 0, label: "Yes, and use auto mode", current: true),
                    ApprovalChoice(index: 1, label: "Yes, manually approve edits", current: false),
                    ApprovalChoice(index: 2, label: "Tell Claude what to change", current: false),
                ],
                acceptsInput: false, hasTextField: true, supportsNote: false,
                createdAtMs: now - 20_000, expiresAtMs: now + 580_000
            ),
        ]
    }

    func machines() -> [Machine] { [machine] }

    func approvalFeed(machineId: String, afterRevision: UInt64) -> ApprovalFeed? {
        ApprovalFeed(link: .connected, revision: 1, missed: false, events: [], pending: pending)
    }

    func flock(machineId: String) async throws -> MachineFlock { throw CoreError.MachineNotFound }

    func decide(machineId: String, approvalId: String, decision: ApprovalDecision, note: String?) async throws -> DecisionOutcome {
        try await Task.sleep(for: .milliseconds(500))
        return .applied(decision: decision, by: "demo")
    }

    func typeText(machineId: String, terminalId: String, text: String) async throws {
        try await Task.sleep(for: .milliseconds(500))
    }
}
#endif
