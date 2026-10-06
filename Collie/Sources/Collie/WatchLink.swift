import CollieCore
import Foundation
import WatchConnectivity

extension WatchState {
    static let maxApprovals = 5
    static let maxAgents = 30

    /// What the phone shows, capped so it fits in an application context.
    init(items: [ApprovalItem], entries: [MachineFlockEntry], allowed: Bool, live: Bool, now: Date) {
        let nowMs = UInt64(max(0, now.timeIntervalSince1970 * 1000))
        let groups = InboxSection.grouped(InboxItem.items(in: entries), now: now)
        let rows = (groups[.working] ?? []).map { ($0, false) } + (groups[.done] ?? []).map { ($0, true) }
        let usage = entries.compactMap { $0.flock?.planUsage }.max { $0.recordedMs < $1.recordedMs }
        self.init(
            approvals: items.filter { $0.approval.expiresAtMs > nowMs }.prefix(Self.maxApprovals).map(WatchApproval.init),
            agents: rows.prefix(Self.maxAgents).map { WatchAgent(item: $0.0, done: $0.1) },
            usage: usage.map {
                WatchUsage(
                    fiveHourUsed: $0.fiveHour.map { min($0.usedPercent, 100) }, fiveHourResetsAtMs: $0.fiveHour?.resetsAtMs,
                    recordedMs: $0.recordedMs
                )
            },
            decisionsAllowed: allowed, live: live
        )
    }
}

/// Unicode scalars, not characters, so a cap also bounds the encoded size.
private func clip(_ text: String, _ limit: Int) -> String {
    String(String.UnicodeScalarView(text.unicodeScalars.prefix(limit)))
}

extension WatchApproval {
    init(item: ApprovalItem) {
        let approval = item.approval
        self.init(
            nodeId: item.machine.nodeId, approvalId: approval.approvalId, agent: clip(approval.agentLabel, 60),
            place: clip([approval.workspaceLabel, item.machine.label].filter { !$0.isEmpty }.joined(separator: " · "), 60),
            command: approval.toolName.map { clip([$0, approval.toolSummary].compactMap { $0 }.joined(separator: ": "), 200) },
            snippet: clip(approval.snippet, 400),
            options: approval.options.compactMap {
                switch $0 {
                case .approve: .approve
                case .deny: .deny
                case .approveAlways, .choose: nil
                }
            },
            choices: approval.choices.prefix(9).map { WatchChoice(index: $0.index, label: clip($0.label, 60)) },
            answeredInTerminal: approval.answeredInTerminal, expiresAtMs: approval.expiresAtMs
        )
    }
}

extension WatchAgent {
    init(item: InboxItem, done: Bool) {
        let agent = item.agent
        let status: Status =
            switch agent.status {
            case .idle: .idle
            case .working: .working
            case .blocked: .blocked
            case .done: .done
            case .unknown: .unknown
            }
        self.init(
            id: "\(item.route.machineId)/\(item.route.terminalId)", title: clip(agent.lastLine ?? agent.displayTitle, 100),
            workspace: item.workspace.map { clip($0, 60) }, machine: clip(item.machine, 60), status: status,
            done: done, activityMs: item.activityMs, contextLeft: agent.contextLeft
        )
    }
}

extension WatchDecision {
    var core: ApprovalDecision {
        switch self {
        case .approve: .approve
        case .deny: .deny
        case .choose(let index): .choose(choice: index)
        }
    }
}

@MainActor
final class WatchLink: NSObject, WCSessionDelegate {
    private var decide: (@MainActor (WatchDecisionRequest) async -> FollowUp)?
    private var lastSent: Data?

    func activate(decide: @escaping @MainActor (WatchDecisionRequest) async -> FollowUp) {
        self.decide = decide
        guard WCSession.isSupported() else { return }
        WCSession.default.delegate = self
        WCSession.default.activate()
    }

    var ready: Bool {
        guard WCSession.isSupported() else { return false }
        let session = WCSession.default
        return session.activationState == .activated && session.isPaired && session.isWatchAppInstalled
    }

    func publish(_ state: WatchState) {
        let encoder = JSONEncoder()
        encoder.outputFormatting = .sortedKeys
        guard let data = try? encoder.encode(state), data != lastSent else { return }
        do {
            try WCSession.default.updateApplicationContext([WatchMessage.state: data])
            lastSent = data
        } catch {}
    }

    /// The last state this phone sent to the watch; nil refuses every decision.
    static func shown() -> WatchState? {
        guard WCSession.isSupported(), let data = WCSession.default.applicationContext[WatchMessage.state] as? Data else { return nil }
        return try? JSONDecoder().decode(WatchState.self, from: data)
    }

    /// Nil only when the setting is on and the request answers an unexpired approval, with an
    /// answer, that this phone showed the watch.
    nonisolated static func refusal(_ request: WatchDecisionRequest, allowed: Bool, shown: WatchState?, now: Date) -> String? {
        guard allowed else {
            return "Decisions from Apple Watch are off. Turn them on in collie Settings on the iPhone. Nothing was sent."
        }
        let nowMs = UInt64(max(0, now.timeIntervalSince1970 * 1000))
        guard let approval = shown?.approvals.first(where: { $0.nodeId == request.nodeId && $0.approvalId == request.approvalId }),
            approval.offers(request.decision), approval.expiresAtMs > nowMs
        else { return "This approval is no longer pending on the iPhone. Nothing was sent." }
        return nil
    }

    nonisolated func session(_ session: WCSession, didReceiveMessage message: [String: Any], replyHandler: @escaping ([String: Any]) -> Void) {
        let data = message[WatchMessage.decide] as? Data
        let reply = Reply(send: replyHandler)
        Task { @MainActor in
            guard let decide, let data, let request = try? JSONDecoder().decode(WatchDecisionRequest.self, from: data) else {
                reply.send([WatchMessage.title: "collie", WatchMessage.body: "Nothing was sent."])
                return
            }
            let followUp = await decide(request)
            reply.send([WatchMessage.title: followUp.title, WatchMessage.body: followUp.body])
        }
    }

    nonisolated func session(_ session: WCSession, activationDidCompleteWith activationState: WCSessionActivationState, error: (any Error)?) {}

    nonisolated func sessionDidBecomeInactive(_ session: WCSession) {}

    /// Switching to another watch deactivates the session; the next one needs a new activation.
    nonisolated func sessionDidDeactivate(_ session: WCSession) {
        WCSession.default.activate()
    }
}

private struct Reply: @unchecked Sendable {
    let send: ([String: Any]) -> Void
}
