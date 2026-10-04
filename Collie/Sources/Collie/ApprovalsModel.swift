import CollieCore
import Foundation
import LocalAuthentication
import Observation

/// The slice of CollieCore the approvals screens use, so the model can run against a fake.
protocol ApprovalCore: AnyObject, Sendable {
    func machines() -> [Machine]
    func approvalFeed(machineId: String, afterRevision: UInt64) -> ApprovalFeed?
    func flock(machineId: String) async throws -> MachineFlock
    func decide(machineId: String, approvalId: String, decision: ApprovalDecision) async throws -> DecisionOutcome
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

    var id: String { approval.approvalId }
}

/// What collied takes from the phone on the prompt an agent is blocked on.
enum BlockedInput: Equatable {
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
    }

    private let core: (any ApprovalCore)?
    private let auth: any Authenticator

    private(set) var items: [ApprovalItem] = []
    private(set) var steps: [String: Step] = [:]
    private(set) var notice: String?
    private(set) var expanded: Set<String> = []
    /// The approval the notice reports on: it leaving the list is the expected result, not a change to clear for.
    private var noticeSubject: String?
    private var noticeTimer: Task<Void, Never>?
    private let noticeLifetime: Duration
    private(set) var refreshing = false
    private(set) var loading: LoadingApproval?
    private var loadTask: Task<Void, Never>?
    var highlighted: String?

    init(
        core: (any ApprovalCore)?, auth: any Authenticator = DeviceOwnerAuthenticator(),
        noticeLifetime: Duration = .seconds(6)
    ) {
        self.core = core
        self.auth = auth
        self.noticeLifetime = noticeLifetime
    }

    /// Local and cheap: reads what the connections already hold.
    func poll() {
        guard let core else { return }
        let next = core.machines().flatMap { machine in
            (core.approvalFeed(machineId: machine.id, afterRevision: .max)?.pending ?? [])
                .map { ApprovalItem(machine: machine, approval: $0) }
        }
        .sorted { ($0.approval.createdAtMs, $0.id) < ($1.approval.createdAtMs, $1.id) }
        guard next != items else { return }
        if next == items.filter({ $0.id != noticeSubject }) {
            noticeSubject = nil
        } else {
            show(nil)
        }
        items = next
        expanded.formIntersection(next.map(\.id))
    }

    func toggleExpanded(_ item: ApprovalItem) {
        if expanded.remove(item.id) == nil { expanded.insert(item.id) }
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
            let wasConnected = core.approvalFeed(machineId: machine.id, afterRevision: .max)?.link == .connected
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
        guard let core, steps[item.id] == nil, item.approval.offers(decision) else { return }
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
                machineId: item.machine.id, approvalId: item.approval.approvalId, decision: decision
            )
            show(outcome.message(agent: item.approval.agentLabel), about: item.id)
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
            "The prompt changed on the Mac. Nothing was sent."
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
