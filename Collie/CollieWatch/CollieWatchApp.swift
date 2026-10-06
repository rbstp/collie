import LocalAuthentication
import SwiftUI
import UserNotifications
import WatchConnectivity
import WidgetKit

@main
struct CollieWatchApp: App {
    @State private var model = WatchModel()

    init() {
        // Mirrored approval alerts would otherwise offer the iPhone's Approve and Deny here, around the phone's watch setting.
        UNUserNotificationCenter.current().setNotificationCategories([
            UNNotificationCategory(identifier: "APPROVAL", actions: [], intentIdentifiers: [])
        ])
    }

    var body: some Scene {
        WindowGroup {
            NavigationStack { WatchHome(model: model) }
        }
        .backgroundTask(.watchConnectivity) { [model] in await model.drain() }
    }
}

@MainActor
@Observable
final class WatchModel: NSObject, WCSessionDelegate {
    private(set) var state: WatchState?
    private(set) var receivedAt: Date?
    private(set) var sending: String?
    /// Decided from this watch: no buttons again while the phone still lists it.
    private(set) var answered: Set<String> = []
    private(set) var notice: (title: String, body: String)?

    override init() {
        super.init()
        guard WCSession.isSupported() else { return }
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

    /// `received` is nil for the context persisted from an earlier launch, whose age is unknown.
    private func reload(received: Date?) {
        guard let data = WCSession.default.receivedApplicationContext[WatchMessage.state] as? Data,
            let next = try? JSONDecoder().decode(WatchState.self, from: data)
        else { return }
        let usageChanged = next.usage != state?.usage
        state = next
        receivedAt = received
        answered.formIntersection(next.approvals.map(\.id))
        if usageChanged {
            if let usage = next.usage { usage.save() } else { WatchUsage.clear() }
            WidgetCenter.shared.reloadTimelines(ofKind: "CollieUsage")
        }
    }

    func decide(_ approval: WatchApproval, _ decision: WatchDecision) async {
        guard state?.decisionsAllowed == true, sending == nil, !answered.contains(approval.id), approval.offers(decision),
            approval.expiresAtMs > UInt64(Date.now.timeIntervalSince1970 * 1000)
        else { return }
        sending = approval.id
        defer { sending = nil }
        notice = nil
        // A fresh context per decision, so one check never covers a later approval.
        let passed = (try? await LAContext().evaluatePolicy(
            .deviceOwnerAuthenticationWithWristDetection, localizedReason: "\(decision.title) \(approval.agent)"
        )) ?? false
        guard passed else {
            notice = (approval.agent, "Needs this watch unlocked, on your wrist, with a passcode and Wrist Detection on. Nothing was sent.")
            return
        }
        guard WCSession.default.isReachable else {
            notice = (approval.agent, "The iPhone is not reachable. Nothing was sent.")
            return
        }
        let request = WatchDecisionRequest(nodeId: approval.nodeId, approvalId: approval.approvalId, decision: decision)
        guard let data = try? JSONEncoder().encode(request) else { return }
        let agent = approval.agent
        let reply: (title: String, body: String, answered: Bool)? = await withCheckedContinuation { continuation in
            WCSession.default.sendMessage(
                [WatchMessage.decide: data],
                replyHandler: { reply in
                    continuation.resume(
                        returning: (
                            reply[WatchMessage.title] as? String ?? agent, reply[WatchMessage.body] as? String ?? "",
                            reply[WatchMessage.answered] as? Bool ?? false
                        )
                    )
                },
                errorHandler: { _ in continuation.resume(returning: nil) }
            )
        }
        if let reply {
            notice = (reply.title, reply.body)
            if reply.answered { answered.insert(approval.id) }
        } else {
            notice = (agent, "No answer from the iPhone. The decision may have been sent; check collie on the iPhone.")
        }
    }

    nonisolated func session(_ session: WCSession, activationDidCompleteWith activationState: WCSessionActivationState, error: (any Error)?) {
        Task { @MainActor in self.reload(received: nil) }
    }

    nonisolated func session(_ session: WCSession, didReceiveApplicationContext applicationContext: [String: Any]) {
        Task { @MainActor in self.reload(received: .now) }
    }
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
        if let state = model.state {
            List {
                if !state.live {
                    Group {
                        if let receivedAt = model.receivedAt {
                            Text("Open collie on the iPhone to update. Received \(receivedAt, format: .relative(presentation: .named)).")
                        } else {
                            Text("Open collie on the iPhone to update")
                        }
                    }
                    .font(.footnote)
                    .foregroundStyle(.secondary)
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
        } else {
            ContentUnavailableView("Open collie on the iPhone", systemImage: "iphone")
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
                Button("Approve") { Task { await model.decide(approval, .approve) } }
                    .buttonStyle(.borderedProminent)
                    .disabled(expired || model.sending != nil)
            }
            if approval.offers(.deny) {
                Button("Deny", role: .destructive) { Task { await model.decide(approval, .deny) } }
                    .disabled(expired || model.sending != nil)
            }
        } else {
            ForEach(approval.choices, id: \.index) { choice in
                Button {
                    Task { await model.decide(approval, .choose(choice.index)) }
                } label: {
                    Text(verbatim: "\(Int(choice.index) + 1). \(choice.label)")
                }
                .disabled(expired || model.sending != nil)
            }
        }
    }
}
