import CollieCore
import Foundation
import WatchConnectivity
import os

extension WatchState {
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
                WatchUsage(fiveHourUsed: $0.fiveHour.map { min($0.usedPercent, 100) }, fiveHourResetsAtMs: $0.fiveHour?.resetsAtMs)
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

extension WatchState {
    /// `shown` with the approvals just listed from each Mac; a Mac that did not answer keeps
    /// those last shown for it.
    static func refreshed(_ shown: WatchState?, listed: [MachineApprovals], machines: [Machine], allowed: Bool, now: Date) -> WatchState {
        let nowMs = UInt64(max(0, now.timeIntervalSince1970 * 1000))
        var state = shown ?? WatchState(approvals: [], agents: [], usage: nil, decisionsAllowed: allowed, live: false)
        var silent: Set<String> = []
        let items = listed.flatMap { entry -> [ApprovalItem] in
            guard let machine = machines.first(where: { $0.id == entry.machineId }) else { return [] }
            guard let approvals = entry.approvals else {
                silent.insert(machine.nodeId)
                return []
            }
            return approvals.map { ApprovalItem(machine: machine, approval: $0, link: .connected) }
        }
        let fresh = items.sorted { ($0.approval.createdAtMs, $0.id) < ($1.approval.createdAtMs, $1.id) }.map(WatchApproval.init)
        let kept = state.approvals.filter { silent.contains($0.nodeId) }
        state.approvals = Array((fresh + kept).filter { $0.expiresAtMs > nowMs }.prefix(Self.maxApprovals))
        state.decisionsAllowed = allowed
        state.live = false
        return state
    }

    /// What the phone shows, but a Mac's approvals change only through its connected link: any
    /// other Mac keeps those last shown for it, as a closed connection's cache is no listing.
    static func published(_ shown: WatchState?, items: [ApprovalItem], entries: [MachineFlockEntry], allowed: Bool, live: Bool, now: Date) -> WatchState {
        let listed = entries.map { entry in
            MachineApprovals(
                machineId: entry.id,
                approvals: entry.flock?.link == .connected ? items.filter { $0.machine.id == entry.id }.map(\.approval) : nil
            )
        }
        var state = WatchState(items: [], entries: entries, allowed: allowed, live: live, now: now)
        state.approvals = refreshed(shown, listed: listed, machines: entries.map(\.machine), allowed: allowed, now: now).approvals
        return state
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

extension BackgroundOutcome {
    /// The answer reached collied, may have, or the approval is gone: the watch need not offer it again.
    var watchAnswered: Bool {
        switch self {
        case .applied, .unconfirmed, .expired, .alreadyResolved, .notFound, .unknownMachine, .unreachable(stage: .decide, message: _): true
        case .superseded, .unreachable, .unauthorized, .failed: false
        }
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
    private var decide: (@MainActor (WatchDecisionRequest) async -> (FollowUp, answered: Bool))?
    private var refresh: (@MainActor () async -> (WatchState, silent: [String])?)?
    private var lastSent: (state: WatchState, at: Date)?
    static let log = Logger(subsystem: "dev.rbstp.collie", category: "watch")

    func activate(
        decide: @escaping @MainActor (WatchDecisionRequest) async -> (FollowUp, answered: Bool),
        refresh: @escaping @MainActor () async -> (WatchState, silent: [String])?
    ) {
        self.decide = decide
        self.refresh = refresh
        guard WCSession.isSupported() else { return }
        WCSession.default.delegate = self
        WCSession.default.activate()
    }

    var ready: Bool {
        guard WCSession.isSupported() else { return false }
        let session = WCSession.default
        return session.activationState == .activated && session.isPaired && session.isWatchAppInstalled
    }

    /// Agent lines and activity change every few seconds while agents work: alone, they go at most once a minute.
    func publish(_ state: WatchState, now: Date = .now) {
        if let lastSent {
            var agentsOnly = state
            agentsOnly.agents = lastSent.state.agents
            guard lastSent.state != state, agentsOnly != lastSent.state || now.timeIntervalSince(lastSent.at) >= 60 else { return }
        }
        send(state, now: now)
    }

    /// Unthrottled: a refresh answers with what the watch may then decide on.
    @discardableResult
    func send(_ state: WatchState, now: Date = .now) -> Bool {
        guard let data = try? JSONEncoder().encode(state) else { return false }
        do {
            try WCSession.default.updateApplicationContext([WatchMessage.state: data])
            lastSent = (state, now)
            Self.log.notice("context: approvals=\(state.approvals.count, privacy: .public) live=\(state.live, privacy: .public)")
            return true
        } catch {
            return false
        }
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
        let refreshing = message[WatchMessage.refresh] as? Bool == true
        let reply = Reply(send: replyHandler)
        Task { @MainActor in
            if refreshing {
                guard let refresh, let refreshed = await refresh(), let data = try? JSONEncoder().encode(refreshed.0) else {
                    reply.send([:])
                    return
                }
                reply.send([WatchMessage.state: data, WatchMessage.silent: refreshed.silent])
                return
            }
            guard let decide, let data, let request = try? JSONDecoder().decode(WatchDecisionRequest.self, from: data) else {
                reply.send([WatchMessage.title: "collie", WatchMessage.body: "Nothing was sent.", WatchMessage.answered: false])
                return
            }
            let (followUp, answered) = await decide(request)
            reply.send([WatchMessage.title: followUp.title, WatchMessage.body: followUp.body, WatchMessage.answered: answered])
        }
    }

    /// A new or reinstalled watch starts with an empty context: send the state again.
    nonisolated func session(_ session: WCSession, activationDidCompleteWith activationState: WCSessionActivationState, error: (any Error)?) {
        Task { @MainActor in self.lastSent = nil }
    }

    nonisolated func sessionWatchStateDidChange(_ session: WCSession) {
        Task { @MainActor in self.lastSent = nil }
    }

    nonisolated func sessionDidBecomeInactive(_ session: WCSession) {}

    /// Switching to another watch deactivates the session; the next one needs a new activation,
    /// and a fresh authenticated opt-in before it can decide.
    nonisolated func sessionDidDeactivate(_ session: WCSession) {
        var prefs = DevicePrefs.load(from: DevicePrefs.file)
        if prefs.watchDecisions {
            prefs.watchDecisions = false
            prefs.save(to: DevicePrefs.file)
        }
        WCSession.default.activate()
    }
}

private struct Reply: @unchecked Sendable {
    let send: ([String: Any]) -> Void
}
