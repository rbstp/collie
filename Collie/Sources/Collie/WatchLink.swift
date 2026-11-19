import CollieCore
import Foundation
import WatchConnectivity
import os

extension WatchState {
    static let maxAgents = 30

    /// What the phone shows, capped so it fits in an application context.
    init(items: [ApprovalItem], entries: [MachineFlockEntry], allowed: Bool, live: Bool, now: Date) {
        let nowMs = now.unixMs
        let groups = InboxSection.grouped(InboxItem.items(in: entries), now: now)
        let rows = ((groups[.needsYou] ?? []) + (groups[.working] ?? [])).map { ($0, false) } + (groups[.done] ?? []).map { ($0, true) }
        self.init(
            approvals: items.filter { $0.approval.expiresAtMs > nowMs }.prefix(Self.maxApprovals).map(WatchApproval.init),
            agents: rows.prefix(Self.maxAgents).map { WatchAgent(item: $0.0, done: $0.1) },
            usage: WatchUsage.latest(entries.compactMap { $0.flock?.planUsage }),
            decisionsAllowed: allowed, live: live
        )
    }
}

extension WatchUsage {
    static func latest(_ plans: [PlanUsage]) -> WatchUsage? {
        guard !plans.isEmpty else { return nil }
        let claude = plans.filter { $0.fiveHour != nil || $0.sevenDay != nil }.max { $0.recordedMs < $1.recordedMs }
        let codex = plans.compactMap(\.codex).max { $0.recordedMs < $1.recordedMs }
        let copilot = plans.compactMap(\.copilot).max { $0.recordedMs < $1.recordedMs }
        return WatchUsage(PlanUsage(
            fiveHour: claude?.fiveHour, sevenDay: claude?.sevenDay, recordedMs: claude?.recordedMs ?? 0, codex: codex, copilot: copilot
        ))
    }

    init(_ plan: PlanUsage) {
        let codex = plan.codex.flatMap { usage -> UInt8? in
            guard usage.limit > 0 else { return nil }
            return UInt8(min(100, (Double(usage.used) / Double(usage.limit) * 100).rounded()))
        }
        let copilot = plan.copilot.flatMap { usage -> UInt8? in
            if usage.unlimited { return 0 }
            guard usage.limit > 0 else { return nil }
            return UInt8(min(100, usage.percent))
        }
        self.init(
            fiveHourUsed: plan.fiveHour.map { min($0.usedPercent, 100) }, fiveHourResetsAtMs: plan.fiveHour?.resetsAtMs,
            sevenDayUsed: plan.sevenDay.map { min($0.usedPercent, 100) }, sevenDayResetsAtMs: plan.sevenDay?.resetsAtMs,
            codexUsed: codex, codexResetsAtMs: plan.codex?.resetsAtMs,
            claudeRecordedMs: plan.fiveHour != nil || plan.sevenDay != nil ? plan.recordedMs : nil,
            codexRecordedMs: plan.codex?.recordedMs,
            copilotUsed: copilot, copilotUnlimited: plan.copilot?.unlimited,
            copilotResetsAtMs: plan.copilot?.resetsAtMs, copilotRecordedMs: plan.copilot?.recordedMs
        )
    }
}

/// Unicode scalars, not characters, so a cap also bounds the encoded size.
private func clip(_ text: String, _ limit: Int) -> String {
    String(String.UnicodeScalarView(text.unicodeScalars.prefix(limit)))
}

extension WatchApproval {
    /// One approval's texts together, in UTF-8 bytes. A permission prompt at every one of collied's caps is under it (its
    /// labels, a command of 600 characters, a reason and a snippet of 200, four options of 300 with 300 of detail: about
    /// 3,600), and five approvals at it still fit an application context beside the agents.
    static let maxTextBytes = 4_096
    /// collied numbers a menu's options 1 to 9.
    static let maxChoices = 9

    /// Every text whole, in the order the watch shows them. The first one that passes the bound is cut at an ellipsis,
    /// nothing after it is sent, and the approval is marked `cut`: a silent prefix once stood above Approve and Deny.
    init(item: ApprovalItem) {
        let approval = item.approval
        // The one ellipsis a cut adds is kept back, so a cut prompt still fits the bound.
        var room = Self.maxTextBytes - "…".utf8.count
        var cut = false
        func fit(_ text: String?) -> String? {
            guard let text, !cut else { return nil }
            guard text.utf8.count > room else {
                room -= text.utf8.count
                return text
            }
            cut = true
            var kept = ""
            for character in text {
                room -= String(character).utf8.count
                if room < 0 { break }
                kept.append(character)
            }
            return kept + "…"
        }
        let agent = fit(approval.agentLabel)
        let place = fit([approval.workspaceLabel, item.machine.label].filter { !$0.isEmpty }.joined(separator: " · "))
        let command = fit(approval.toolName.map { [$0, approval.toolSummary].compactMap { $0 }.joined(separator: ": ") })
        let reason = fit(approval.reason)
        let snippet = fit(approval.snippet)
        let choices = approval.choices.prefix(Self.maxChoices).compactMap { choice in
            fit(choice.label).map { WatchChoice(index: choice.index, label: $0, detail: fit(choice.detail)) }
        }
        self.init(
            nodeId: item.machine.nodeId, approvalId: approval.approvalId, agent: agent ?? "", place: place ?? "",
            command: command, reason: reason, snippet: snippet ?? "",
            options: approval.options.compactMap {
                switch $0 {
                case .approve: .approve
                case .deny: .deny
                case .approveAlways, .choose: nil
                }
            },
            choices: choices, cut: cut || approval.choices.count > Self.maxChoices,
            answeredInTerminal: approval.answeredInTerminal, expiresAtMs: approval.expiresAtMs
        )
    }
}

extension WatchState {
    /// `shown` with the approvals and the newest plan usage just listed from each Mac; a Mac
    /// that did not answer keeps the approvals last shown for it.
    static func refreshed(_ shown: WatchState?, listed: [MachineApprovals], machines: [Machine], allowed: Bool, now: Date) -> WatchState {
        let nowMs = now.unixMs
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
        if let usage = WatchUsage.latest(listed.compactMap(\.planUsage)) {
            state.usage = usage
        } else if !listed.isEmpty && listed.allSatisfy({ $0.approvals != nil }) {
            state.usage = nil
        }
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
        self.init(
            id: "\(item.route.machineId)/\(item.route.terminalId)", title: clip(agent.lastLine ?? agent.displayTitle, 100),
            workspace: item.workspace.map { clip($0, 60) }, machine: clip(item.machine, 60),
            status: Status(rawValue: AgentActivityStatus(agent.status).rawValue) ?? .unknown,
            done: done, activityMs: agent.activityMs, contextLeft: agent.contextLeft
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

    func publish(_ state: WatchState, now: Date = .now) {
        guard Self.due(state, lastSent: lastSent, reachable: WCSession.default.isReachable, now: now) else { return }
        send(state, now: now)
    }

    /// Agent lines and activity change every few seconds while agents work: alone, they go only
    /// while the watch app is reachable, at most once a minute, as the context can wake the watch.
    nonisolated static func due(_ state: WatchState, lastSent: (state: WatchState, at: Date)?, reachable: Bool, now: Date) -> Bool {
        guard let lastSent else { return true }
        var agentsOnly = state
        agentsOnly.agents = lastSent.state.agents
        return lastSent.state != state && (agentsOnly != lastSent.state || reachable && now.timeIntervalSince(lastSent.at) >= 60)
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
        let nowMs = now.unixMs
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
        Task { @MainActor in
            DevicePrefs.turnOffWatchDecisions(in: DevicePrefs.file)
            WCSession.default.activate()
        }
    }
}

private struct Reply: @unchecked Sendable {
    let send: ([String: Any]) -> Void
}
