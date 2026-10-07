import LocalAuthentication
import SwiftUI
import UserNotifications
import WatchConnectivity
import WatchKit
import WidgetKit

@main
struct CollieWatchApp: App {
    @WKApplicationDelegateAdaptor(WatchDelegate.self) private var delegate
    @Environment(\.scenePhase) private var scenePhase

    var body: some Scene {
        let model = delegate.model
        WindowGroup {
            NavigationStack { WatchHome(model: model) }
        }
        .onChange(of: scenePhase) { _, phase in
            if phase == .active { Task { await model.refreshIfStale() } }
            if phase == .background { model.scheduleBackgroundRefresh() }
        }
        .backgroundTask(.watchConnectivity) { [model] in await model.drain() }
        .backgroundTask(.appRefresh(WatchModel.backgroundRefresh)) { [model] _ in
            // Scheduled first, so a run the system ends early still leaves the next one.
            await model.scheduleBackgroundRefresh()
            await model.refreshIfStale(timeout: .seconds(10))
        }
    }
}

/// Any response to an alert naming an approval, whichever button, only opens it here: a
/// decision always goes through the watch path, gated by the phone's setting.
@MainActor
final class WatchDelegate: NSObject, WKApplicationDelegate, UNUserNotificationCenterDelegate {
    let model = WatchModel()

    func applicationDidFinishLaunching() {
        let center = UNUserNotificationCenter.current()
        center.setNotificationCategories([UNNotificationCategory(identifier: "APPROVAL", actions: [], intentIdentifiers: [])])
        center.delegate = self
    }

    nonisolated func userNotificationCenter(
        _ center: UNUserNotificationCenter,
        didReceive response: UNNotificationResponse,
        withCompletionHandler completionHandler: @escaping @Sendable () -> Void
    ) {
        let info = response.notification.request.content.userInfo
        let nodeId = info["node_id"] as? String
        let approvalId = info["approval_id"] as? String
        Task { @MainActor in
            if let nodeId, let approvalId { self.model.open(nodeId: nodeId, approvalId: approvalId) }
            completionHandler()
        }
    }

    /// In front, the alert still shows, and a locked phone publishes nothing: ask it.
    nonisolated func userNotificationCenter(
        _ center: UNUserNotificationCenter,
        willPresent notification: UNNotification,
        withCompletionHandler completionHandler: @escaping @Sendable (UNNotificationPresentationOptions) -> Void
    ) {
        let info = notification.request.content.userInfo
        let approval = info["node_id"] is String && info["approval_id"] is String
        completionHandler([.banner, .sound, .list])
        if approval { Task { @MainActor in await self.model.refresh() } }
    }
}

@MainActor
@Observable
final class WatchModel: NSObject, WCSessionDelegate {
    private(set) var state: WatchState?
    private(set) var sending: String?
    /// Decided from this watch: no buttons again while the phone still lists it.
    private(set) var answered: Set<String> = []
    private(set) var notice: (title: String, body: String)?
    private(set) var refreshing = false
    private(set) var refreshFailed = false
    /// The Macs the last refresh did not reach; empty with `refreshFailed` when no reply came.
    private var silent: Set<String> = []
    private var refreshedAt: Date?
    private var again = false
    /// The approval an alert opened, shown once the state has it.
    var opened: WatchApprovalKey?

    override init() {
        super.init()
        WCSession.default.delegate = self
        WCSession.default.activate()
    }

    /// Keeps a background launch alive until the pending context is delivered.
    func drain() async {
        let session = WCSession.default
        if session.activationState == .notActivated { session.activate() }
        for _ in 0..<50 {
            guard session.activationState != .activated || session.hasContentPending else { return }
            try? await Task.sleep(for: .milliseconds(200))
        }
    }

    func open(nodeId: String, approvalId: String) {
        opened = WatchApprovalKey(nodeId: nodeId, approvalId: approvalId)
        Task { await refresh() }
    }

    func couldNotRefresh(_ nodeId: String) -> Bool {
        refreshFailed && (silent.isEmpty || silent.contains(nodeId))
    }

    /// A raised wrist makes the app active again: each refresh wakes the phone and dials every Mac.
    func refreshIfStale(timeout: Duration = .seconds(25)) async {
        if let refreshedAt, Date.now.timeIntervalSince(refreshedAt) < 60 { return }
        await refresh(timeout: timeout)
    }

    static let backgroundRefresh = "refresh"

    /// watchOS budgets about 4 background refreshes an hour to an app whose complication is on the active face.
    func scheduleBackgroundRefresh() {
        WKApplication.shared().scheduleBackgroundRefresh(
            withPreferredDate: .now.addingTimeInterval(15 * 60), userInfo: Self.backgroundRefresh as NSString
        ) { _ in }
    }

    /// Asks the phone, which answers even while locked; on no answer the last state stays.
    func refresh(timeout: Duration = .seconds(25)) async {
        guard !refreshing else {
            again = true
            return
        }
        refreshing = true
        defer { refreshing = false }
        repeat {
            again = false
            await ask(timeout: timeout)
        } while again
        // As on the phone, back to the list: a rebuilt approval for that agent shows there.
        if let key = opened, let state, !couldNotRefresh(key.nodeId), state.approvals.count < WatchState.maxApprovals,
            !state.approvals.contains(where: { $0.nodeId == key.nodeId && $0.approvalId == key.approvalId })
        {
            opened = nil
            notice = ("collie", "This approval is no longer pending.")
        }
    }

    private func ask(timeout: Duration) async {
        let session = WCSession.default
        for _ in 0..<10 where session.activationState != .activated || !session.isReachable {
            try? await Task.sleep(for: .milliseconds(200))
        }
        guard session.activationState == .activated, session.isReachable else {
            silent = []
            refreshFailed = true
            return
        }
        let (replies, continuation) = AsyncStream.makeStream(of: RefreshReply?.self)
        session.sendMessage(
            [WatchMessage.refresh: true],
            replyHandler: { @Sendable reply in
                continuation.yield(
                    RefreshReply(state: reply[WatchMessage.state] as? Data, silent: reply[WatchMessage.silent] as? [String] ?? [])
                )
                continuation.finish()
            },
            errorHandler: { @Sendable _ in
                continuation.yield(nil)
                continuation.finish()
            }
        )
        let deadline = Task {
            try? await Task.sleep(for: timeout)
            continuation.yield(nil)
            continuation.finish()
        }
        defer { deadline.cancel() }
        var reply: RefreshReply?
        for await first in replies {
            reply = first
            break
        }
        guard let reply, let data = reply.state, let next = try? JSONDecoder().decode(WatchState.self, from: data) else {
            silent = []
            refreshFailed = true
            return
        }
        apply(next)
        silent = Set(reply.silent)
        refreshFailed = !silent.isEmpty
        if !refreshFailed { refreshedAt = .now }
    }

    /// `fresh` is false for the context persisted from an earlier launch.
    private func reload(fresh: Bool) {
        guard let data = WCSession.default.receivedApplicationContext[WatchMessage.state] as? Data,
            let next = try? JSONDecoder().decode(WatchState.self, from: data)
        else { return }
        apply(next)
        if fresh && next.live {
            silent = []
            refreshFailed = false
        }
    }

    private func apply(_ next: WatchState) {
        let usageChanged = next.usage != state?.usage
        state = next
        answered.formIntersection(next.approvals.map(\.id))
        if usageChanged {
            if let usage = next.usage { usage.save() } else { WatchUsage.clear() }
            WidgetCenter.shared.reloadTimelines(ofKind: WatchUsage.widgetKind)
        }
    }

    /// True once the phone answered that this approval need not be offered again.
    func decide(_ approval: WatchApproval, _ decision: WatchDecision) async -> Bool {
        guard sending == nil, approval.canDecide(decision, allowed: state?.decisionsAllowed == true, answered: answered, now: .now)
        else { return false }
        sending = approval.id
        defer { sending = nil }
        notice = nil
        // A fresh context per decision, so one check never covers a later approval.
        let passed = (try? await LAContext().evaluatePolicy(
            .deviceOwnerAuthenticationWithWristDetection, localizedReason: "\(decision.title) \(approval.agent)"
        )) ?? false
        guard passed else {
            notice = (approval.agent, "Needs this watch unlocked, on your wrist, with a passcode and Wrist Detection on. Nothing was sent.")
            return false
        }
        guard WCSession.default.isReachable else {
            notice = (approval.agent, "The iPhone is not reachable. Nothing was sent.")
            return false
        }
        let request = WatchDecisionRequest(nodeId: approval.nodeId, approvalId: approval.approvalId, decision: decision)
        guard let data = try? JSONEncoder().encode(request) else { return false }
        let agent = approval.agent
        let reply: (title: String, body: String, answered: Bool)? = await withCheckedContinuation { continuation in
            WCSession.default.sendMessage(
                [WatchMessage.decide: data],
                replyHandler: { @Sendable reply in
                    continuation.resume(
                        returning: (
                            reply[WatchMessage.title] as? String ?? agent, reply[WatchMessage.body] as? String ?? "",
                            reply[WatchMessage.answered] as? Bool ?? false
                        )
                    )
                },
                errorHandler: { @Sendable _ in continuation.resume(returning: nil) }
            )
        }
        if let reply {
            notice = (reply.title, reply.body)
            if reply.answered {
                answered.insert(approval.id)
                if opened == WatchApprovalKey(nodeId: approval.nodeId, approvalId: approval.approvalId) { opened = nil }
            }
        } else {
            notice = (agent, "No answer from the iPhone. The decision may have been sent; check collie on the iPhone.")
        }
        return reply?.answered == true
    }

    nonisolated func session(_ session: WCSession, activationDidCompleteWith activationState: WCSessionActivationState, error: (any Error)?) {
        Task { @MainActor in self.reload(fresh: false) }
    }

    nonisolated func session(_ session: WCSession, didReceiveApplicationContext applicationContext: [String: Any]) {
        Task { @MainActor in self.reload(fresh: true) }
    }
}

private struct RefreshReply: Sendable {
    let state: Data?
    let silent: [String]
}

struct WatchApprovalKey: Hashable {
    let nodeId: String
    let approvalId: String
}

extension WatchDecision {
    fileprivate var title: String {
        switch self {
        case .approve: "Approve"
        case .deny: "Deny"
        case .choose(let index): "Choose option \(Int(index) + 1)"
        }
    }
}

extension WatchAgent.Status {
    fileprivate var symbol: String {
        switch self {
        case .working: "circle.dotted.circle"
        case .blocked: "exclamationmark.circle.fill"
        case .done: "checkmark.circle.fill"
        case .idle: "circle.fill"
        case .unknown: "questionmark.circle"
        }
    }

    fileprivate var color: Color {
        switch self {
        case .working: .blue
        case .blocked: .red
        case .done: .green
        case .idle: .gray
        case .unknown: .secondary
        }
    }
}

private func date(ms: UInt64) -> Date {
    Date(timeIntervalSince1970: TimeInterval(ms) / 1000)
}

private struct WatchHome: View {
    let model: WatchModel

    var body: some View {
        content.navigationDestination(item: Binding(get: { model.opened }, set: { model.opened = $0 })) { key in
            OpenedApproval(model: model, key: key)
        }
    }

    @ViewBuilder
    private var content: some View {
        if let state = model.state {
            List {
                if model.refreshFailed {
                    Text("Couldn't refresh from the iPhone").font(.footnote).foregroundStyle(.secondary)
                }
                if let notice = model.notice {
                    Text(verbatim: "\(notice.title): \(notice.body)").font(.footnote)
                }
                Section("Approvals") {
                    let pending = state.approvals.filter { !model.answered.contains($0.id) }
                    if pending.isEmpty {
                        Text("No pending approvals").foregroundStyle(.secondary)
                    }
                    ForEach(pending) { approval in
                        NavigationLink {
                            ApprovalDetail(model: model, approval: approval)
                        } label: {
                            VStack(alignment: .leading) {
                                Text(verbatim: approval.agent).font(.headline).lineLimit(1)
                                Text(verbatim: approval.place).font(.caption).foregroundStyle(.secondary).lineLimit(1)
                                Text(timerInterval: Date.now...max(.now, date(ms: approval.expiresAtMs)), countsDown: true)
                                    .font(.caption.monospacedDigit())
                                    .foregroundStyle(.secondary)
                            }
                        }
                    }
                }
                AgentSection(title: "Working", agents: state.agents.filter { !$0.done })
                AgentSection(title: "Done", agents: state.agents.filter(\.done))
            }
            .navigationTitle("collie")
        } else if model.refreshing {
            ProgressView()
        } else {
            ContentUnavailableView("Open collie on the iPhone", systemImage: "iphone")
        }
    }
}

private struct OpenedApproval: View {
    let model: WatchModel
    let key: WatchApprovalKey

    var body: some View {
        if let approval = model.state?.approvals.first(where: { $0.nodeId == key.nodeId && $0.approvalId == key.approvalId }) {
            ApprovalDetail(model: model, approval: approval)
        } else if model.refreshing {
            ProgressView()
        } else if model.couldNotRefresh(key.nodeId) {
            Text("Couldn't refresh from the iPhone. Open collie on the iPhone to see this approval.").font(.footnote)
        } else if model.state?.approvals.count ?? 0 >= WatchState.maxApprovals {
            Text("Not shown on the watch. Open collie on the iPhone.").font(.footnote)
        } else {
            Text("This approval is no longer pending.").font(.footnote)
        }
    }
}

private struct AgentSection: View {
    let title: String
    let agents: [WatchAgent]

    var body: some View {
        if !agents.isEmpty {
            Section(title) {
                ForEach(agents) { agent in
                    HStack(alignment: .firstTextBaseline) {
                        Image(systemName: agent.status.symbol).foregroundStyle(agent.status.color)
                        VStack(alignment: .leading, spacing: 2) {
                            Text(verbatim: agent.title).lineLimit(2)
                            Text(verbatim: [agent.workspace, agent.machine].compactMap { $0 }.joined(separator: " · "))
                                .font(.caption)
                                .foregroundStyle(.secondary)
                                .lineLimit(1)
                            HStack {
                                TimelineView(.everyMinute) { context in
                                    Text(date(ms: agent.activityMs).formatted(.relative(presentation: .numeric, unitsStyle: .narrow)))
                                }
                                if let left = agent.contextLeft {
                                    Text("\(left)%")
                                }
                            }
                            .font(.caption2.monospacedDigit())
                            .foregroundStyle(.secondary)
                        }
                    }
                }
            }
        }
    }
}

private struct ApprovalDetail: View {
    let model: WatchModel
    let approval: WatchApproval
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        let expiry = date(ms: approval.expiresAtMs)
        ScrollView {
            VStack(alignment: .leading, spacing: 8) {
                Text(verbatim: approval.agent).font(.headline)
                Text(verbatim: approval.place).font(.caption).foregroundStyle(.secondary)
                Text(timerInterval: Date.now...max(.now, expiry), countsDown: true)
                    .font(.caption.monospacedDigit())
                    .foregroundStyle(.secondary)
                if let command = approval.command {
                    Text(verbatim: command).font(.footnote.monospaced())
                }
                Text(verbatim: approval.snippet).font(.footnote).foregroundStyle(.secondary)
                TimelineView(.explicit([expiry])) { _ in
                    actions(expired: Date.now >= expiry)
                }
                if let notice = model.notice {
                    Text(verbatim: "\(notice.title): \(notice.body)").font(.footnote)
                }
            }
        }
    }

    @ViewBuilder
    private func actions(expired: Bool) -> some View {
        if approval.answeredInTerminal {
            Text("Answer it in the terminal on the machine").font(.footnote).foregroundStyle(.secondary)
        } else if model.state?.decisionsAllowed != true {
            Text("Read only. Turn on Decide from Apple Watch in collie Settings on the iPhone.")
                .font(.footnote)
                .foregroundStyle(.secondary)
        } else if model.answered.contains(approval.id) {
            EmptyView()
        } else if model.sending == approval.id {
            ProgressView()
        } else if approval.options.isEmpty && approval.choices.isEmpty {
            Text("Answer it on the iPhone").font(.footnote).foregroundStyle(.secondary)
        } else if !approval.options.isEmpty {
            if approval.offers(.approve) {
                Button("Approve") { decide(.approve) }
                    .buttonStyle(.borderedProminent)
                    .tint(.green)
                    .disabled(expired || model.sending != nil)
            }
            if approval.offers(.deny) {
                Button("Deny", role: .destructive) { decide(.deny) }
                    .disabled(expired || model.sending != nil)
            }
        } else {
            ForEach(approval.choices, id: \.index) { choice in
                Button {
                    decide(.choose(choice.index))
                } label: {
                    Text(verbatim: "\(Int(choice.index) + 1). \(choice.label)")
                }
                .disabled(expired || model.sending != nil)
            }
        }
    }

    /// Back to the list once answered, where the notice reports it; on failure the buttons stay.
    private func decide(_ decision: WatchDecision) {
        Task { if await model.decide(approval, decision) { dismiss() } }
    }
}
