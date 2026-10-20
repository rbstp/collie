import ActivityKit
import CollieCore
import CryptoKit
import Foundation
import SwiftUI
import Synchronization

#if DEBUG
/// `--terminal-demo <file>`: the Agents list over fake agents, each agent screen showing the
/// first "text" string of a herdr JSON response (or the raw file), with no Mac. Debug builds only.
/// `--follow-demo <terminal id>` also follows that agent, which starts its Live Activity, and
/// gives a blocked one the approval fields collied would push. `--approval-demo` shows that
/// approval's card on the blocked agent's screen.
struct AgentDemo: View {
    let core: DemoAgentCore
    private let followed: String?
    @State private var approvals: ApprovalsModel?
    @State private var follows: FollowModel
    @State private var opening: AgentRoute?
    private let accent = DevicePrefs.load(from: DevicePrefs.file).accent

    init?(arguments: [String]) {
        guard let flag = arguments.firstIndex(of: "--terminal-demo"), flag + 1 < arguments.count,
            let data = FileManager.default.contents(atPath: arguments[flag + 1])
        else { return nil }
        let json = try? JSONSerialization.jsonObject(with: data)
        core = DemoAgentCore(
            snapshot: json.flatMap(Self.firstText) ?? String(decoding: data, as: UTF8.self),
            connectionError: ProcessInfo.processInfo.environment["COLLIE_DEMO_CONNECTION_ERROR"],
            subagentDemo: arguments.contains("--subagent-demo")
        )
        _approvals = State(initialValue: arguments.contains("--approval-demo") ? ApprovalsModel(core: core, dismissResolved: { _, _ in }) : nil)
        _follows = State(initialValue: FollowModel(core: core, approvals: nil, file: nil))
        followed = arguments.firstIndex(of: "--follow-demo").flatMap { $0 + 1 < arguments.count ? arguments[$0 + 1] : nil }
    }

    var body: some View {
        TabView {
            Tab("Agents", systemImage: "square.grid.2x2") {
                FlockScreen(core: core, machines: [DemoAgentCore.machine], approvals: approvals, follows: follows, opening: $opening)
            }
            Tab("Machines", systemImage: "desktopcomputer") {
                NavigationStack {
                    List {
                        MachineRow(machine: DemoAgentCore.machine, flock: core.cachedFlock(machineId: DemoAgentCore.machine.id)) {
                            Text("Machine settings")
                        }
                        .listRowInsets(EdgeInsets(top: 4, leading: 16, bottom: 4, trailing: 16))
                        .listRowBackground(Color.clear)
                        .listRowSeparator(.hidden)
                    }
                    .listStyle(.plain)
                    .scrollContentBackground(.hidden)
                    .navigationTitle("Machines")
                }
            }
            Tab("Settings", systemImage: "gearshape") { Color.clear }
        }
        .tint(accent.color)
        .accentColor(accent.color)
        .environment(\.appAccent, accent.color)
        .task {
            approvals?.poll()
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
        for activity in Activity<AgentActivityAttributes>.activities where activity.content.state.opened(attributes: activity.attributes).status == .blocked {
            var state = activity.content.state.opened(attributes: activity.attributes)
            state.approvalId = approvalId
            state.enc = sealed.combined.base64EncodedString()
            if let content = FollowModel.activityContent(state, attributes: activity.attributes) {
                await activity.update(content)
            }
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

#endif

final class DemoAgentCore: ActivityCore, ApprovalCore {
    static let machine = Machine(id: "demo", label: "MacBook Pro", host: "mac.example.ts.net", port: 8457, nodeId: "nDEMO", kind: .mac, key: "")

    let snapshot: String
    private let connectionError: String?
    private let agents: [AgentSummary]
    private let usage: PlanUsage
    private let stars = Mutex<[String]>([])

    private struct Simulation {
        var resolved = false
        var revision: UInt64 = 1
        var output: [String: String] = [:]
        var messages: [String: [HistoryMessage]] = [:]
    }

    private let simulation = Mutex(Simulation())
    let isSample: Bool

    convenience init() {
        self.init(snapshot: "Sample workspace: all actions are simulated locally.\r\n18 tests passed.\r\n", connectionError: nil, isSample: true)
    }

    #if DEBUG
    convenience init(snapshot: String, connectionError: String? = nil, subagentDemo: Bool = false) {
        self.init(snapshot: snapshot, connectionError: connectionError, isSample: false, subagentDemo: subagentDemo)
    }
    #endif

    private var screenshotDemo: Bool {
        #if DEBUG
        return ProcessInfo.processInfo.environment["COLLIE_DEMO_SCREENSHOTS"] != nil
        #else
        return false
        #endif
    }

    private init(snapshot: String, connectionError: String?, isSample: Bool, subagentDemo: Bool = false) {
        self.isSample = isSample
        self.snapshot = snapshot
        self.connectionError = connectionError
        let now = Date.now.unixMs
        usage = PlanUsage(
            fiveHour: UsageWindow(usedPercent: 38, resetsAtMs: now + 7_980_000),
            sevenDay: UsageWindow(usedPercent: 61, resetsAtMs: now + 277_200_000),
            recordedMs: now - 20_000
        )
        agents = [
            AgentSummary(
                terminalId: "term_demo", workspaceId: "ws_collie", kind: "claude", name: nil, title: "fix the build",
                status: .working, statusSinceMs: now - 135_000, cwd: "/Users/demo/collie",
                lastLine: "Running the approval tests again after the settle fix.", contextLeft: 72,
                lastPrompt: "the approval tests are flaky, fix them", lastActivityMs: now - 4_000,
                subagents: subagentDemo ? ["Review approval flow", "Run approval tests", "Check retry handling", "Inspect test fixtures", "Review documentation"].enumerated().map { index, title in
                    SubagentSummary(agentType: "general-purpose", title: title, activity: title, startedAtMs: now - UInt64(5 + index) * 60_000, tokens: 12_000, calledTools: true)
                } : []
            ),
            AgentSummary(
                terminalId: "term_2", workspaceId: "ws_collie", kind: "claude", name: nil, title: "add approval tests",
                status: .blocked, statusSinceMs: now - 42_000, cwd: "/Users/demo/collie",
                lastLine: "I need to run cargo test to check the new cases.", contextLeft: 38,
                lastPrompt: "add approval tests for the deny path", lastActivityMs: now - 42_000,
                subagents: subagentDemo ? [
                    SubagentSummary(agentType: "general-purpose", title: "Review deny path", activity: "Reading approval tests", startedAtMs: now - 180_000, tokens: 4_000, calledTools: true)
                ] : []
            ),
            AgentSummary(
                terminalId: "term_3", workspaceId: "ws_site", kind: "claude", name: nil, title: "update the docs",
                status: .idle, statusSinceMs: now - 900_000, cwd: "/Users/demo/website",
                lastLine: "The docs now describe the inbox and the context ring.", contextLeft: 12,
                lastPrompt: "update the docs for the new release", lastActivityMs: now - 900_000
            ),
            AgentSummary(
                terminalId: "term_4", workspaceId: "ws_site", kind: "codex", name: nil, title: "dark mode",
                status: .done, statusSinceMs: now - 3_600_000, cwd: "/Users/demo/website",
                lastLine: "Dark mode follows the system setting on every page.", contextLeft: 91,
                lastPrompt: "add dark mode to the website", lastActivityMs: now - 3_600_000
            ),
            AgentSummary(
                terminalId: "term_5", workspaceId: "ws_api", kind: "claude", name: nil, title: "rate limiter",
                status: .idle, statusSinceMs: now - 259_200_000, cwd: "/Users/demo/api",
                lastLine: "The rate limiter is merged.", contextLeft: 55,
                lastPrompt: "merge the rate limiter PR", lastActivityMs: now - 259_200_000
            ),
            AgentSummary(
                terminalId: "term_6", workspaceId: "ws_api", kind: "copilot", name: nil, title: "triage issues",
                status: .idle, statusSinceMs: now - 7_200_000, cwd: "/Users/demo/api", lastLine: nil
            ),
        ]
    }

    func machines() -> [Machine] { [Self.machine] }

    func cachedFlock(machineId: String) -> MachineFlock? { demoFlock }

    func reconnect(machineId: String) throws {}

    func agentView(machineId: String, terminalId: String, afterRevision: UInt64) -> AgentView? {
        let revision = simulation.withLock { $0.revision }
        return AgentView(
            link: .connected, lastError: nil,
            agent: currentAgents.first { $0.terminalId == terminalId },
            output: afterRevision < revision ? read(terminalId) : nil,
            outputRevision: revision
        )
    }

    func watchAgent(machineId: String, terminalId: String?, lines: UInt16) async throws {}

    func agentRead(machineId: String, terminalId: String, source: TerminalSource, lines: UInt16?) async throws -> TerminalSnapshot {
        read(terminalId)
    }

    func agentChanges(machineId: String, terminalId: String, root: String?, branch: Bool) async throws -> GitChanges {
        guard isSample else { throw CoreError.NotImplemented }
        return GitChanges(root: "/Users/demo/collie", branch: "sample-approval-tests", base: "master", files: [
            GitFile(path: "tests/approvals.rs", oldPath: nil, section: "unstaged", status: "modified", additions: 1, deletions: 1, hash: nil)
        ], truncated: false, worktrees: [])
    }

    func agentDiff(machineId: String, terminalId: String, root: String, path: String, section: String) async throws -> GitDiff {
        guard isSample else { throw CoreError.NotImplemented }
        return GitDiff(patch: "diff --git a/tests/approvals.rs b/tests/approvals.rs\n--- a/tests/approvals.rs\n+++ b/tests/approvals.rs\n@@ -1 +1 @@\n-sleep(Duration::from_millis(200)).await;\n+wait_for_approval_state().await;\n", truncated: false)
    }
    func agentDraft(machineId: String, terminalId: String) async throws -> String? {
        #if DEBUG
        return terminalId == "term_demo" ? ProcessInfo.processInfo.environment["COLLIE_DEMO_MAC_DRAFT"] : nil
        #else
        return nil
        #endif
    }

    func approvalFeed(machineId: String) -> ApprovalFeed? {
        if simulation.withLock({ $0.resolved }) { return ApprovalFeed(link: .connected, pending: []) }
        let now = Date.now.unixMs
        return ApprovalFeed(link: .connected, pending: [
            PendingApproval(
                approvalId: "ap_demo", terminalId: "term_2", agentLabel: "Claude Code", workspaceLabel: "collie",
                snippet: (1...8).map { "test approvals::case_\($0) ... ok" }.joined(separator: "\n"),
                toolName: "Bash", toolSummary: "cargo test -p collied --test approvals -- --nocapture",
                options: [.approve, .approveAlways, .deny], choices: [], acceptsInput: true, hasTextField: false,
                supportsNote: true, createdAtMs: now, expiresAtMs: now + 600_000
            ),
        ])
    }

    func decide(machineId: String, approvalId: String, decision: ApprovalDecision, note: String?) async throws -> DecisionOutcome {
        guard isSample, machineId == Self.machine.id, approvalId == "ap_demo" else { throw CoreError.NotImplemented }
        let applied = simulation.withLock { state in
            guard !state.resolved else { return false }
            state.resolved = true
            return true
        }
        guard applied else { return .expired }
        record(terminalId: "term_2", text: "Simulated \(decision == .deny ? "denial" : "approval"). No command was run.")
        return .applied(decision: decision, by: "sample")
    }

    func agentHistory(machineId: String, terminalId: String, before: String?, after: String?, query: String?) async throws -> HistoryPage {
        if isSample {
            let extra = simulation.withLock { $0.messages[terminalId, default: []] }
            var messages = Self.screenshotHistory(terminalId) + extra
            if let query { messages = messages.filter { $0.text.localizedCaseInsensitiveContains(query) } }
            if before != nil { messages = [] }
            if let after, let index = messages.firstIndex(where: { $0.id == after }) { messages = Array(messages.dropFirst(index + 1)) }
            return HistoryPage(session: "sample", available: true, unavailableReason: nil, reset: false, messages: messages, before: nil, after: messages.last?.id ?? after, more: false, omitted: false)
        }
        if screenshotDemo {
            let messages = after == nil && before == nil ? Self.screenshotHistory(terminalId) : []
            return HistoryPage(session: "demo", available: true, unavailableReason: nil, reset: false, messages: messages, before: nil, after: "end", more: false, omitted: false)
        }
        let messages = after == nil && before == nil ? (1...20).flatMap { turn in
            [
                HistoryMessage(id: "u\(turn)", role: "user", text: "Run the approval tests again, round \(turn).", tool: nil, callId: nil, truncated: false, parentCallId: nil),
                HistoryMessage(
                    id: "a\(turn)", role: "assistant",
                    text: "Round \(turn): all 18 approval tests pass. The fixed 200 ms wait is now a poll on the approval state with a five second deadline, so a slow run still passes and a real hang still fails.",
                    tool: nil, callId: nil, truncated: false, parentCallId: nil
                ),
            ]
        } : []
        return HistoryPage(session: "demo", available: true, unavailableReason: nil, reset: false, messages: messages, before: nil, after: "end", more: false, omitted: false)
    }

    func agentLatest(machineId: String, terminalId: String) async throws -> HistoryPage {
        let page = try await agentHistory(machineId: machineId, terminalId: terminalId, before: nil, after: nil, query: nil)
        var counts: [String: Int] = [:]
        let messages = page.messages.reversed().filter { message in
            guard message.tool == nil, ["user", "assistant"].contains(message.role) else { return false }
            counts[message.role, default: 0] += 1
            return counts[message.role, default: 0] <= 2
        }
        return HistoryPage(
            session: page.session, available: true, unavailableReason: nil, reset: false, messages: messages.reversed(),
            before: nil, after: nil, more: false, omitted: false
        )
    }

    private static func screenshotHistory(_ terminalId: String) -> [HistoryMessage] {
        func message(_ id: String, _ role: String, _ text: String, tool: String? = nil, call: String? = nil) -> HistoryMessage {
            HistoryMessage(id: id, role: role, text: text, tool: tool, callId: call, truncated: false, parentCallId: nil)
        }
        if terminalId == "term_2" {
            return [
                message("u1", "user", "add approval tests for the deny path"),
                message("a1", "assistant", "I'll add three cases to `crates/collied/tests/approvals.rs`:\n\n- deny with a reason\n- deny after the prompt changed\n- deny from the lock screen"),
                message("t1", "assistant", "crates/collied/tests/approvals.rs", tool: "Update", call: "c1"),
                message("r1", "tool", "Added 3 tests", tool: "Result", call: "c1"),
                message("a2", "assistant", "I need to run cargo test to check the new cases."),
            ]
        }
        return [
            message("u1", "user", "the approval tests are flaky, fix them"),
            message("a1", "assistant", "I'll read the approval tests first to see where the timing depends on an idle machine."),
            message("t1", "assistant", "crates/collied/tests/approvals.rs", tool: "Read", call: "c1"),
            message("r1", "tool", "Read 412 lines", tool: "Result", call: "c1"),
            message("a2", "assistant", "The tests wait a fixed 200 ms for the hook to settle, which is too short on a busy machine. The fix:\n\n- poll the approval state instead of sleeping\n- give up after a five second deadline"),
            message("t2", "assistant", "crates/collied/tests/approvals.rs", tool: "Update", call: "c2"),
            message("r2", "tool", "14 additions and 6 removals", tool: "Result", call: "c2"),
            message("t3", "assistant", "cargo nextest run -p collied --test approvals", tool: "Bash", call: "c3"),
            message("r3", "tool", "18 tests run: 18 passed", tool: "Result", call: "c3"),
            message("a3", "assistant", "All 18 approval tests pass:\n\n```\n18 tests run: 18 passed\n```\n\nI'll run them twenty times under load before I commit."),
            message("u2", "user", "commit it once the loop passes"),
            message("a4", "assistant", "Will do. The loop is on run 7 of 20 and every run has passed so far."),
        ]
    }

    func prompt(machineId: String, terminalId: String, text: String, expectedDraft: String?) async throws {
        if isSample { record(terminalId: terminalId, text: "Simulated response to: \(text)\nThe sample action completed locally. No command was run.") }
        else { try await Task.sleep(for: .milliseconds(500)) }
    }

    func resendPrompt(machineId: String, terminalId: String, text: String, expectedDraft: String?) async throws {
        try await prompt(machineId: machineId, terminalId: terminalId, text: text, expectedDraft: expectedDraft)
    }

    func sendKeys(machineId: String, terminalId: String, keys: [AgentKey]) async throws {
        if isSample { record(terminalId: terminalId, text: "Simulated terminal keys received. No keys were sent to a machine.") }
    }

    func typeText(machineId: String, terminalId: String, text: String) async throws {
        if isSample { record(terminalId: terminalId, text: "Simulated terminal input: \(text)") }
    }

    func focus(machineId: String, terminalId: String) async throws {}
    func scrollBottom(machineId: String, terminalId: String) async throws {}
    func answerNotice(machineId: String, terminalId: String, digit: UInt8, label: String) async throws {}
    func slashDraft(machineId: String, terminalId: String, command: String, expectedDraft: String?) async throws {}

    func star(machineId: String, terminalId: String, starred: Bool) async throws {
        stars.withLock { stars in
            stars.removeAll { $0 == terminalId }
            if starred { stars.append(terminalId) }
        }
    }

    func watchPullRequest(machineId: String, terminalId: String, everyMinutes: UInt32?) async throws {
        throw CoreError.NotImplemented
    }

    func closeWorkspace(machineId: String, workspaceId: String, confirm: Bool) async throws {
        throw CoreError.NotImplemented
    }

    func closePane(machineId: String, terminalId: String, confirm: Bool) async throws {
        throw CoreError.NotImplemented
    }

    func archiveTask(machineId: String, terminalId: String, confirm: Bool) async throws -> String {
        throw CoreError.NotImplemented
    }

    func taskOptions(machineId: String) async throws -> TaskOptions {
        guard isSample || screenshotDemo else { throw CoreError.NotImplemented }
        return TaskOptions(agents: ["claude", "codex", "copilot"], defaultAgent: "claude", recentCwds: ["/Users/demo/collie", "/Users/demo/website"], roots: ["/Users/demo"])
    }

    func taskFolders(machineId: String, path: String) async throws -> TaskFolders {
        throw CoreError.NotImplemented
    }

    func taskNew(machineId: String, cwd: String, agent: String, prompt: String, label: String?, newFolder: String?, ultracode: Bool) async throws -> TaskStarted {
        throw CoreError.NotImplemented
    }

    func taskWorktrees(machineId: String, cwd: String) async throws -> WorktreeListing { throw CoreError.NotImplemented }
    func taskWorktreeCreate(machineId: String, cwd: String, branch: String, agent: String, prompt: String, label: String?, ultracode: Bool) async throws -> TaskStarted { throw CoreError.NotImplemented }
    func taskWorktreeOpen(machineId: String, cwd: String, path: String, agent: String, prompt: String, label: String?, ultracode: Bool) async throws -> TaskStarted { throw CoreError.NotImplemented }
    func taskWorktreePullRequest(machineId: String, cwd: String, number: UInt64, agent: String, prompt: String, label: String?, ultracode: Bool) async throws -> TaskStarted { throw CoreError.NotImplemented }
    func taskClone(machineId: String, cwd: String, repository: String, agent: String, prompt: String, label: String?, ultracode: Bool) async throws -> TaskStarted { throw CoreError.NotImplemented }
    func taskSessions(machineId: String, cwd: String) async throws -> [TaskSession] { throw CoreError.NotImplemented }
    func taskResume(machineId: String, cwd: String, agent: String, sessionId: String, prompt: String, label: String?) async throws -> TaskStarted { throw CoreError.NotImplemented }

    func flock(machineId: String) async throws -> MachineFlock { demoFlock }

    func usageCosts(machineId: String, range: CostRange, utcOffsetMinutes: Int16) async throws -> UsageCosts {
        let count = switch range {
        case .today: 1
        case .week: 7
        case .month: 30
        }
        let calendar = Calendar.current
        let today = calendar.startOfDay(for: .now)
        let pattern = [8.42, 12.95, 6.10, 15.31, 9.84, 11.27, 7.56]
        let days = (0..<count).map { index in
            let date = calendar.date(byAdding: .day, value: index - count + 1, to: today) ?? today
            let parts = calendar.dateComponents([.year, .month, .day], from: date)
            return DayCost(
                date: String(format: "%04d-%02d-%02d", parts.year ?? 0, parts.month ?? 0, parts.day ?? 0),
                costUsd: pattern[index % pattern.count]
            )
        }
        let total = days.map(\.costUsd).reduce(0, +)
        let scale = UInt64(count)
        let models = [
            ModelCost(agent: "claude", model: "claude-opus-5-5", tokens: CostTokens(input: 1_200 * scale, output: 210_000 * scale, cacheRead: 9_800_000 * scale, cacheWrite: 310_000 * scale), costUsd: total * 0.72, cacheSavingsUsd: total * 4.1),
            ModelCost(agent: "codex", model: "gpt-6.1-sol", tokens: CostTokens(input: 1_100_000 * scale, output: 160_000 * scale, cacheRead: 6_400_000 * scale, cacheWrite: 0), costUsd: total * 0.2, cacheSavingsUsd: total * 0.9),
            ModelCost(agent: "copilot", model: "claude-opus-5.5", tokens: CostTokens(input: 300 * scale, output: 40_000 * scale, cacheRead: 1_900_000 * scale, cacheWrite: 90_000 * scale), costUsd: total * 0.08, cacheSavingsUsd: total * 0.4),
        ]
        return UsageCosts(
            scanning: false,
            tokens: models.reduce(into: CostTokens(input: 0, output: 0, cacheRead: 0, cacheWrite: 0)) { sum, m in
                sum.input += m.tokens.input
                sum.output += m.tokens.output
                sum.cacheRead += m.tokens.cacheRead
                sum.cacheWrite += m.tokens.cacheWrite
            },
            costUsd: total,
            cacheSavingsUsd: total * 5.4,
            days: days,
            models: models
        )
    }

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

    func terminalChallenge(machineId: String, terminalId: String) async throws -> Data {
        throw CoreError.TerminalsDisabled
    }

    func terminalGrant(machineId: String, terminalId: String, signature: Data) async throws {
        throw CoreError.TerminalsDisabled
    }

    func watchTerminal(machineId: String, terminalId: String, lines: UInt16) async throws {}

    func terminalRun(machineId: String, terminalId: String, text: String) async throws {
        throw CoreError.TerminalsDisabled
    }

    func terminalSendKeys(machineId: String, terminalId: String, keys: [AgentKey]) async throws {
        throw CoreError.TerminalsDisabled
    }

    func registerActivityToken(machineId: String, activityId: String, terminalId: String, tokenHex: String, bindingId: String, relayGrant: RelayPushGrant?) throws {
        print("demo: activity \(activityId) token \(tokenHex.prefix(8))… for \(terminalId)")
    }

    func endActivity(machineId: String, activityId: String) throws {}

    private func record(terminalId: String, text: String) {
        simulation.withLock { state in
            state.revision += 1
            state.output[terminalId, default: ""] += "\r\n" + text.replacingOccurrences(of: "\n", with: "\r\n") + "\r\n"
            state.messages[terminalId, default: []].append(HistoryMessage(id: "sample-\(state.revision)", role: "assistant", text: text, tool: nil, callId: nil, truncated: false, parentCallId: nil))
        }
    }

    private var currentAgents: [AgentSummary] {
        guard simulation.withLock({ $0.resolved }) else { return agents }
        return agents.map { agent in
            guard agent.terminalId == "term_2" else { return agent }
            var agent = agent
            agent.status = .done
            agent.lastLine = "Sample decision applied locally. No command was run."
            return agent
        }
    }

    private var demoFlock: MachineFlock {
        MachineFlock(
            machine: Self.machine, link: connectionError == nil ? .connected : .waiting, lastError: connectionError,
            details: MachineDetails(name: "MacBook Pro", nodeId: "nDEMO", herdrSession: "default"),
            workspaces: [
                WorkspaceSummary(workspaceId: "ws_collie", label: "collie", number: 1, status: .blocked, cwd: "/Users/demo/collie"),
                WorkspaceSummary(workspaceId: "ws_site", label: "website", number: 2, status: .done, cwd: "/Users/demo/website"),
                WorkspaceSummary(workspaceId: "ws_api", label: "api", number: 3, status: .idle, cwd: "/Users/demo/api"),
            ],
            agents: currentAgents, approvalsCount: simulation.withLock { $0.resolved ? 0 : 1 }, planUsage: usage, starred: stars.withLock { $0 }
        )
    }

    private func read(_ terminalId: String) -> TerminalSnapshot {
        let preview = (isSample || screenshotDemo) ? Self.screenshotPreviews[terminalId]?.replacingOccurrences(of: "\n", with: "\r\n") : nil
        return TerminalSnapshot(terminalId: terminalId, source: .recent, ansi: (preview ?? snapshot) + simulation.withLock { $0.output[terminalId, default: ""] }, truncated: false)
    }

    private static let screenshotPreviews = [
        "term_2": """
            ⏺ Update(crates/collied/tests/approvals.rs)
              ⎿  Added 3 tests

            ⏺ I need to run cargo test to check the new cases.

             Bash command

               cargo test -p collied --test approvals

             Do you want to proceed?
             ❯ 1. Yes
               2. No
            """,
        "term_3": """
            ⏺ Update(docs/inbox.md)
              ⎿  Updated inbox.md with 9 additions

            ⏺ Update(docs/release-notes.md)
              ⎿  Updated release-notes.md with 24 additions

            ⏺ The docs now describe the inbox and the context
              ring.
            """,
        "term_4": """
            • Edited src/styles/theme.css (+18 -2)

            • Ran npm run build
              └ Built 14 pages in 1.2s

            ─ Worked for 4m 12s ──────────────────────────

            • Dark mode follows the system setting on every
              page.
            """,
        "term_5": """
            ⏺ Bash(gh pr checks 42)
              ⎿  All checks were successful

            ⏺ Bash(gh pr merge 42 --squash)
              ⎿  Merged pull request #42

            ⏺ The rate limiter is merged.
            """,
        "term_6": """
            ● List open issues
              └ 12 issues

            ● Labeled 5 issues as bug and 3 as enhancement.
              The other 4 need more detail from their
              authors.
            """,
    ]
}
