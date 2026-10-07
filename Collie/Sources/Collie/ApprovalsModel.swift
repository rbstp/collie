import CollieCore
import Foundation
import LocalAuthentication
import Observation
import UserNotifications

/// The slice of CollieCore the approvals screens use, so the model can run against a fake.
protocol ApprovalCore: AnyObject, Sendable {
    func machines() -> [Machine]
    func approvalFeed(machineId: String) -> ApprovalFeed?
    func flock(machineId: String) async throws -> MachineFlock
    func decide(machineId: String, approvalId: String, decision: ApprovalDecision, note: String?) async throws -> DecisionOutcome
    func typeText(machineId: String, terminalId: String, text: String) async throws
}

extension CollieCore: ApprovalCore {}

protocol Authenticator: Sendable {
    func authenticate(reason: String) async -> Bool
}

/// A fresh context per decision, so one unlock never covers a later approval.
struct DeviceOwnerAuthenticator: Authenticator {
    func authenticate(reason: String) async -> Bool {
        (try? await LAContext().evaluatePolicy(.deviceOwnerAuthentication, localizedReason: reason)) ?? false
    }
}

struct ApprovalItem: Identifiable, Equatable {
    let machine: Machine
    let approval: PendingApproval
    let link: LinkPhase

    var id: String { approval.approvalId }

    /// Connecting stays reachable, so a decision made during the resume grace still goes out.
    var unreachable: Bool { [.waiting, .unavailable, .offline, .stopped].contains(link) }
}

/// What collied takes from the phone on the prompt an agent is blocked on.
enum BlockedInput: Equatable {
    /// Nothing: the prompt is answered in the terminal on the machine.
    case terminal
    case optionsOnly
    case keys
    /// Keys, and typed text into the menu's free-text option.
    case keysAndText
}

struct LoadingApproval: Equatable {
    let link: ApprovalLink
    let machine: Machine
    var phase: LinkPhase?
}

@MainActor
@Observable
final class ApprovalsModel {
    enum Step: Equatable {
        case authenticating(ApprovalDecision)
        case sending(ApprovalDecision)
        case typing
    }

    private let core: (any ApprovalCore)?
    private let auth: any Authenticator

    private(set) var items: [ApprovalItem] = []
    private(set) var steps: [String: Step] = [:]
    private(set) var notice: String?
    private(set) var expanded: Set<String> = []
    /// Unsent notes and plan feedback, by approval.
    var drafts: [String: String] = [:]
    /// Approvals whose note field is shown; a hidden note is never sent.
    private(set) var noting: Set<String> = []
    /// The approval the notice reports on: it leaving the list is the expected result, not a change to clear for.
    private var noticeSubject: String?
    private var noticeTimer: Task<Void, Never>?
    private let noticeLifetime: Duration
    private(set) var refreshing = false
    private(set) var loading: LoadingApproval?
    private var loadTask: Task<Void, Never>?
    /// Pending approval ids per connected machine at its last notification sweep.
    private var swept: [String: Set<String>] = [:]
    private let dismissResolved: @Sendable (_ nodeId: String, _ pending: Set<String>) -> Void
    var highlighted: String?

    init(
        core: (any ApprovalCore)?, auth: any Authenticator = DeviceOwnerAuthenticator(),
        noticeLifetime: Duration = .seconds(6),
        dismissResolved: @escaping @Sendable (_ nodeId: String, _ pending: Set<String>) -> Void = ApprovalsModel.removeDelivered
    ) {
        self.core = core
        self.auth = auth
        self.noticeLifetime = noticeLifetime
        self.dismissResolved = dismissResolved
    }

    /// Removes this machine's delivered approval alerts whose approval is no longer pending,
    /// however it was answered. Follow-ups carry no approval_id and stay.
    nonisolated static func removeDelivered(nodeId: String, pending: Set<String>) {
        Task {
            let center = UNUserNotificationCenter.current()
            let stale = await center.deliveredNotifications().filter { note in
                let info = note.request.content.userInfo
                guard info["node_id"] as? String == nodeId, let id = info["approval_id"] as? String else { return false }
                return !pending.contains(id)
            }
            center.removeDeliveredNotifications(withIdentifiers: stale.map(\.request.identifier))
        }
    }

    /// Local and cheap: reads what the connections already hold.
    func poll() {
        guard let core else { return }
        let nowMs = Date.now.unixMs
        var connected: Set<String> = []
        let next = core.machines().flatMap { machine -> [ApprovalItem] in
            guard let feed = core.approvalFeed(machineId: machine.id) else { return [] }
            let live = feed.pending.filter { $0.expiresAtMs > nowMs }
            // Only a connected machine's list is current: a cached one may miss resolutions.
            if feed.link == .connected {
                connected.insert(machine.id)
                let ids = Set(live.map(\.approvalId))
                if swept[machine.id] != ids {
                    swept[machine.id] = ids
                    dismissResolved(machine.nodeId, ids)
                }
            }
            return live.map { ApprovalItem(machine: machine, approval: $0, link: feed.link) }
        }
        .sorted { ($0.approval.createdAtMs, $0.id) < ($1.approval.createdAtMs, $1.id) }
        swept = swept.filter { connected.contains($0.key) }
        guard next != items else { return }
        let pending = next.map(\.approval)
        if pending == items.filter({ $0.id != noticeSubject }).map(\.approval) {
            noticeSubject = nil
        } else if pending != items.map(\.approval) {
            show(nil)
        }
        items = next
        let ids = Set(next.map(\.id))
        expanded.formIntersection(ids)
        noting.formIntersection(ids)
        drafts = drafts.filter { ids.contains($0.key) }
    }

    func toggleExpanded(_ item: ApprovalItem) {
        if expanded.remove(item.id) == nil { expanded.insert(item.id) }
    }

    func toggleNote(_ item: ApprovalItem) {
        if noting.remove(item.id) == nil { noting.insert(item.id) }
    }

    /// The note Approve or Deny sends: nil while the field is hidden or blank.
    func note(for item: ApprovalItem) -> String? {
        guard noting.contains(item.id) else { return nil }
        let text = drafts[item.id, default: ""].trimmingCharacters(in: .whitespacesAndNewlines)
        return text.isEmpty ? nil : text
    }

    private func show(_ text: String?, about subject: String? = nil) {
        notice = text
        noticeSubject = text == nil ? nil : subject
        noticeTimer?.cancel()
        guard text != nil else { return }
        noticeTimer = Task { [weak self, noticeLifetime] in
            try? await Task.sleep(for: noticeLifetime)
            if !Task.isCancelled { self?.show(nil) }
        }
    }

    /// Fetches a fresh snapshot from every Mac, which also opens connections that do not exist yet.
    func refresh() async {
        guard let core, !refreshing else { return }
        refreshing = true
        defer { refreshing = false }
        await withTaskGroup(of: Void.self) { group in
            for machine in core.machines() {
                group.addTask { _ = try? await core.flock(machineId: machine.id) }
            }
        }
        poll()
    }

    func items(machineId: String, terminalId: String) -> [ApprovalItem] {
        items.filter { $0.machine.id == machineId && $0.approval.terminalId == terminalId }
    }

    /// What collied takes from the phone while this agent is blocked: nil when no approval blocks it.
    func blockedInput(machineId: String, terminalId: String) -> BlockedInput? {
        let blocking = items(machineId: machineId, terminalId: terminalId).map(\.approval)
        guard !blocking.isEmpty else { return nil }
        guard !blocking.allSatisfy(\.answeredInTerminal) else { return .terminal }
        guard blocking.allSatisfy(\.acceptsInput) else { return .optionsOnly }
        return blocking.allSatisfy(\.hasTextField) ? .keysAndText : .keys
    }

    @discardableResult
    func open(_ link: ApprovalLink) -> Task<Void, Never>? {
        highlighted = link.approvalId
        show(nil)
        loadTask?.cancel()
        loading = nil
        poll()
        guard let core, !items.contains(where: { $0.id == link.approvalId }),
            let machine = core.machines().first(where: { $0.nodeId == link.nodeId })
        else { return nil }
        loading = LoadingApproval(link: link, machine: machine)
        loadTask = Task { await load(link, machine: machine, core: core) }
        return loadTask
    }

    /// Until the approval shows up, or a snapshot fetched over an already connected link
    /// proves it is gone: on a cold launch the feed is empty only because nothing is connected yet.
    private func load(_ link: ApprovalLink, machine: Machine, core: any ApprovalCore) async {
        while !Task.isCancelled {
            let wasConnected = core.approvalFeed(machineId: machine.id)?.link == .connected
            let flock = try? await core.flock(machineId: machine.id)
            guard !Task.isCancelled else { return }
            poll()
            if items.contains(where: { $0.id == link.approvalId }) { break }
            if wasConnected, flock?.link == .connected {
                show("This approval is no longer pending.")
                break
            }
            if let phase = flock?.link { loading?.phase = phase }
            try? await Task.sleep(for: .milliseconds(500))
        }
        if !Task.isCancelled { loading = nil }
    }

    /// Nothing reaches the core unless the device owner authenticates first.
    func decide(_ item: ApprovalItem, _ decision: ApprovalDecision) async {
        let note = note(for: item)
        guard let core, steps[item.id] == nil, item.approval.offers(decision),
            note == nil || item.approval.takesNote(with: decision)
        else { return }
        if let note {
            do { try checkNote(note: note, decision: decision) } catch {
                show(describe(error), about: item.id)
                return
            }
        }
        steps[item.id] = .authenticating(decision)
        show(nil)
        guard await auth.authenticate(reason: decision.reason(agent: item.approval.agentLabel)) else {
            steps[item.id] = nil
            show("Not authenticated. Nothing was sent.", about: item.id)
            return
        }
        steps[item.id] = .sending(decision)
        defer { steps[item.id] = nil }
        do {
            let outcome = try await core.decide(
                machineId: item.machine.id, approvalId: item.approval.approvalId, decision: decision, note: note
            )
            show(outcome.message(agent: item.approval.agentLabel), about: item.id)
        } catch {
            show(describe(error), about: item.id)
        }
        poll()
    }

    /// Typed into the plan's "Tell Claude what to change"; collied moves the cursor there and never sends shift+tab.
    func sendFeedback(_ item: ApprovalItem) async {
        let text = drafts[item.id, default: ""].trimmingCharacters(in: .whitespacesAndNewlines)
        guard let core, steps[item.id] == nil, item.approval.takesFeedback, !text.isEmpty else { return }
        // Feedback is typed into whatever text option is on screen, so only for a plan still pending.
        poll()
        guard items.contains(where: { $0.id == item.id }) else {
            show("This plan is no longer waiting for an answer.", about: item.id)
            return
        }
        steps[item.id] = .typing
        show(nil)
        defer { steps[item.id] = nil }
        do {
            try await core.typeText(machineId: item.machine.id, terminalId: item.approval.terminalId, text: text)
            drafts[item.id] = nil
            show("Sent your feedback to \(item.approval.agentLabel).", about: item.id)
        } catch {
            show(describe(error), about: item.id)
        }
        poll()
    }
}

extension PendingApproval {
    /// A menu option is only ever chosen where collied offers no Approve/Deny.
    func offers(_ decision: ApprovalDecision) -> Bool {
        if case .choose(let choice) = decision {
            return options.isEmpty && choices.contains { $0.index == choice }
        }
        return options.contains(decision)
    }

    /// collied takes a note only with Approve or Deny, on a prompt that showed "Tab to amend".
    func takesNote(with decision: ApprovalDecision) -> Bool {
        supportsNote && (decision == .approve || decision == .deny)
    }

    /// A plan prompt: options only for keys, but its free-text option takes typed feedback.
    var takesFeedback: Bool { hasTextField && !acceptsInput }

    /// collied takes no answer from the phone: another agent kind (Codex, Copilot) or a prompt it does not read.
    var answeredInTerminal: Bool { options.isEmpty && choices.isEmpty && !acceptsInput && !hasTextField }
}

extension DecisionOutcome {
    func message(agent: String) -> String {
        switch self {
        case .applied(let decision, _):
            "\(decision.pastTense): \(agent). The agent moved on."
        case .unconfirmed(let decision, _):
            "\(decision.pastTense), not confirmed: the keys were sent but \(agent) still looks blocked."
        case .expired:
            "This approval expired. Nothing was sent."
        case .superseded:
            "The prompt changed on the machine. Nothing was sent."
        case .unknown:
            "collied answered with an outcome this version does not know."
        }
    }
}

enum Countdown {
    static func string(untilMs: UInt64, now: Date) -> String {
        let seconds = Int(untilMs / 1000) - Int(now.timeIntervalSince1970)
        guard seconds > 0 else { return "expired" }
        return String(format: "%d:%02d", seconds / 60, seconds % 60)
    }
}
