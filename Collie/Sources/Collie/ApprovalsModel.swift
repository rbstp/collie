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
    private(set) var refreshing = false
    private(set) var loading: LoadingApproval?
    private var loadTask: Task<Void, Never>?
    var highlighted: String?

    init(core: (any ApprovalCore)?, auth: any Authenticator = DeviceOwnerAuthenticator()) {
        self.core = core
        self.auth = auth
    }

    /// Local and cheap: reads what the connections already hold.
    func poll() {
        guard let core else { return }
        let next = core.machines().flatMap { machine in
            (core.approvalFeed(machineId: machine.id, afterRevision: .max)?.pending ?? [])
                .map { ApprovalItem(machine: machine, approval: $0) }
        }
        .sorted { ($0.approval.createdAtMs, $0.id) < ($1.approval.createdAtMs, $1.id) }
        if next != items { items = next }
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

    @discardableResult
    func open(_ link: ApprovalLink) -> Task<Void, Never>? {
        highlighted = link.approvalId
        notice = nil
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
                notice = "This approval is no longer pending."
                break
            }
            if let phase = flock?.link { loading?.phase = phase }
            try? await Task.sleep(for: .milliseconds(500))
        }
        if !Task.isCancelled { loading = nil }
    }

    /// Nothing reaches the core unless the device owner authenticates first.
    func decide(_ item: ApprovalItem, _ decision: ApprovalDecision) async {
        guard let core, steps[item.id] == nil, item.approval.options.contains(decision) else { return }
        steps[item.id] = .authenticating(decision)
        notice = nil
        guard await auth.authenticate(reason: "\(decision.title) \(item.approval.agentLabel)") else {
            steps[item.id] = nil
            notice = "Not authenticated. Nothing was sent."
            return
        }
        steps[item.id] = .sending(decision)
        defer { steps[item.id] = nil }
        do {
            let outcome = try await core.decide(
                machineId: item.machine.id, approvalId: item.approval.approvalId, decision: decision
            )
            notice = outcome.message(agent: item.approval.agentLabel)
        } catch {
            notice = describe(error)
        }
        poll()
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
