import ActivityKit
import AppIntents
import CollieCore
import Foundation
import Observation
import SwiftUI
import UIKit
import os

/// The slice of CollieCore Live Activities use, so the model can run against a fake.
protocol ActivityCore: FlockCore {
    func registerActivityToken(machineId: String, activityId: String, terminalId: String, tokenHex: String) throws
    func endActivity(machineId: String, activityId: String) throws
}

extension CollieCore: ActivityCore {}

struct FollowedAgent: Codable, Hashable, Sendable {
    let machineId: String
    let terminalId: String

    init(_ route: AgentRoute) {
        machineId = route.machineId
        terminalId = route.terminalId
    }

    var route: AgentRoute { AgentRoute(machineId: machineId, terminalId: terminalId) }
}

/// An activity whose token went to a Mac. Ids only, never tokens: iOS can end an activity while
/// the app is not running, and its Mac must still be told to stop pushing to it.
struct RegisteredActivity: Codable, Hashable, Sendable {
    let machineId: String
    let activityId: String
}

/// Agents followed on the Lock Screen, kept on this device in the state dir. Every agent starts unfollowed.
struct FollowList: Codable, Equatable {
    /// ActivityKit refuses more than about five activities per app.
    static let limit = 5
    static let file: URL? = try? StateDirectory.prepare().appending(path: "follows.json")

    private(set) var agents: [FollowedAgent] = []
    private(set) var registered: [RegisteredActivity] = []

    static func load(from file: URL?) -> FollowList {
        file.flatMap { try? Data(contentsOf: $0) }.flatMap { try? JSONDecoder().decode(Self.self, from: $0) } ?? FollowList()
    }

    func save(to file: URL?) {
        guard let file, let data = try? JSONEncoder().encode(self) else { return }
        try? data.write(to: file, options: .atomic)
    }

    func contains(_ agent: FollowedAgent) -> Bool { agents.contains(agent) }

    var isFull: Bool { agents.count >= Self.limit }

    @discardableResult
    mutating func add(_ agent: FollowedAgent) -> Bool {
        guard !contains(agent) else { return true }
        guard !isFull else { return false }
        agents.append(agent)
        return true
    }

    mutating func remove(_ agent: FollowedAgent) {
        agents.removeAll { $0 == agent }
    }

    /// False when it was already remembered.
    mutating func remember(_ activity: RegisteredActivity) -> Bool {
        guard !registered.contains(activity) else { return false }
        registered.append(activity)
        return true
    }

    /// False when it was not remembered.
    mutating func forget(activityId: String) -> Bool {
        let count = registered.count
        registered.removeAll { $0.activityId == activityId }
        return registered.count != count
    }
}

/// Live Activities for followed agents. Activities start from the foreground only; collied
/// updates them by push through the token each one registers with its Mac.
@MainActor
@Observable
final class FollowModel {
    nonisolated static let staleAfter: TimeInterval = 15 * 60

    private let core: (any ActivityCore)?
    private let approvals: ApprovalsModel?
    private let file: URL?
    private(set) var list: FollowList
    private(set) var enabled: Bool
    var notice: String?
    @ObservationIgnored private var watchers: [String: Task<Void, Never>] = [:]
    /// Restarts are tried once per foreground, so a refused request is not retried on every poll.
    @ObservationIgnored private var attempted: Set<FollowedAgent> = []
    @ObservationIgnored private let log = Logger(subsystem: "dev.rbstp.collie", category: "live-activity")

    init(core: (any ActivityCore)?, approvals: ApprovalsModel?, file: URL? = FollowList.file) {
        self.core = core
        self.approvals = approvals
        self.file = file
        list = FollowList.load(from: file)
        enabled = ActivityAuthorizationInfo().areActivitiesEnabled
    }

    func isFollowing(_ route: AgentRoute) -> Bool {
        list.contains(FollowedAgent(route))
    }

    /// An activity started or watched in this run has no push token for its Mac yet.
    var awaitingToken: Bool {
        watchers.keys.contains { id in !list.registered.contains { $0.activityId == id } }
    }

    func follow(_ route: AgentRoute) {
        let agent = FollowedAgent(route)
        guard !list.contains(agent) else { return }
        guard !list.isFull else {
            notice = "You can follow up to \(FollowList.limit) agents on the Lock Screen. Stop following one first."
            return
        }
        enabled = ActivityAuthorizationInfo().areActivitiesEnabled
        guard enabled else {
            notice = "Live Activities are off for collie. Turn them on in Settings to follow agents on the Lock Screen."
            return
        }
        guard case let (machine, title, state)? = content(for: agent) else {
            notice = "This agent's status is not known yet. Try again once its machine is connected."
            return
        }
        do {
            try start(agent, machine: machine, title: title, state: state)
        } catch {
            notice = "Could not start the Live Activity: \(error.localizedDescription)"
            return
        }
        list.add(agent)
        list.save(to: file)
        attempted.insert(agent)
    }

    func unfollow(_ route: AgentRoute) {
        let agent = FollowedAgent(route)
        list.remove(agent)
        list.save(to: file)
        for activity in activities(for: agent) {
            end(activity, dismissal: .immediate)
        }
        endDead()
    }

    /// On every foreground: hand each running activity's token to its Mac again, end activities
    /// no longer followed, and restart followed ones iOS ended or an older build started.
    func foreground() {
        enabled = ActivityAuthorizationInfo().areActivitiesEnabled
        attempted = []
        for activity in Activity<AgentActivityAttributes>.activities where activity.isLive {
            let agent = FollowedAgent(AgentRoute(machineId: activity.attributes.machineId, terminalId: activity.attributes.terminalId))
            guard list.contains(agent), !activity.attributes.isOutdated else {
                end(activity, dismissal: .immediate)
                continue
            }
            watch(activity)
            if let token = activity.pushToken { register(activity, token: token) }
        }
        endDead()
        sync()
    }

    /// Tells the Macs about activities iOS ended or the user dismissed while the app was not
    /// running, so they stop pushing to them.
    private func endDead() {
        let live = Set(Activity<AgentActivityAttributes>.activities.filter(\.isLive).map(\.id))
        for activity in list.registered where !live.contains(activity.activityId) {
            forget(machineId: activity.machineId, activityId: activity.activityId)
        }
    }

    /// Follows the flock the app already polls: local updates while in the foreground, the
    /// activity of a closed pane or a removed Mac ends.
    func sync() {
        guard let core, !list.agents.isEmpty else { return }
        let machines = Set(core.machines().map(\.id))
        for agent in list.agents {
            let flock = core.cachedFlock(machineId: agent.machineId)
            let gone = !machines.contains(agent.machineId)
                || (flock?.link == .connected && flock?.details != nil
                    && flock?.agents.contains { $0.terminalId == agent.terminalId } == false)
            if gone {
                list.remove(agent)
                list.save(to: file)
                for activity in activities(for: agent) {
                    end(activity, dismissal: .after(.now.addingTimeInterval(60)))
                }
                continue
            }
            guard case let (machine, title, state)? = content(for: agent) else { continue }
            let live = activities(for: agent)
            if live.isEmpty {
                guard enabled, !attempted.contains(agent), UIApplication.shared.applicationState == .active else { continue }
                attempted.insert(agent)
                endDead()
                do {
                    try start(agent, machine: machine, title: title, state: state)
                } catch {
                    log.error("restart: \(error.localizedDescription, privacy: .public)")
                }
                continue
            }
            for activity in live {
                let current = activity.content
                let state = state.keepingApproval(of: current.state)
                let staleSoon = current.staleDate.map { $0 < .now.addingTimeInterval(Self.staleAfter / 3) } ?? true
                guard current.state != state || staleSoon else { continue }
                let content = Self.activityContent(state)
                let id = activity.id
                Task.detached { await Self.live(id)?.update(content) }
            }
        }
    }

    nonisolated static func activityContent(
        _ state: AgentActivityAttributes.ContentState
    ) -> ActivityContent<AgentActivityAttributes.ContentState> {
        ActivityContent(state: state, staleDate: .now.addingTimeInterval(staleAfter), relevanceScore: state.relevance)
    }

    /// After a decision applied: drops the buttons and shows the status the agent moves to, at
    /// once. Dated a second before the tap so any update collied pushes afterwards, whose
    /// timestamp is whole seconds, is never older and always replaces it.
    nonisolated static func resolve(_ link: ApprovalLink, approved: Bool, tappedAt: Date) async {
        guard
            let activity = Activity<AgentActivityAttributes>.activities.first(where: {
                $0.isLive && $0.attributes.nodeId == link.nodeId && $0.content.state.approvalId == link.approvalId
            })
        else { return }
        var state = activity.content.state
        state.status = approved ? .working : .idle
        state.statusSince = .now
        state.approvalId = nil
        state.enc = nil
        state.progress = nil
        state.approvals = max(0, state.approvals - 1)
        await activity.update(activityContent(state), alertConfiguration: nil, timestamp: tappedAt.addingTimeInterval(-1))
    }

    /// Shows `progress` instead of the buttons on the live activity holding this approval, and
    /// returns that activity's agent title and terminal id.
    @discardableResult
    nonisolated static func show(progress: String?, on link: ApprovalLink) async -> (title: String, terminalId: String)? {
        guard
            let activity = Activity<AgentActivityAttributes>.activities.first(where: {
                $0.isLive && $0.attributes.nodeId == link.nodeId && $0.content.state.pendingApproval == link.approvalId
            })
        else { return nil }
        var state = activity.content.state
        state.progress = progress
        await activity.update(activityContent(state))
        return (state.title, activity.attributes.terminalId)
    }

    private func content(for agent: FollowedAgent) -> (Machine, String, AgentActivityAttributes.ContentState)? {
        guard let flock = core?.cachedFlock(machineId: agent.machineId), flock.link == .connected,
            let summary = flock.agents.first(where: { $0.terminalId == agent.terminalId })
        else { return nil }
        let workspace = flock.workspaces.first { $0.workspaceId == summary.workspaceId }?.label
        let approvals = approvals?.items(machineId: agent.machineId, terminalId: agent.terminalId).count ?? 0
        return (
            flock.machine, summary.activityTitle,
            AgentActivityAttributes.ContentState(agent: summary, workspace: workspace, approvals: approvals)
        )
    }

    private func start(_ agent: FollowedAgent, machine: Machine, title: String, state: AgentActivityAttributes.ContentState) throws {
        let attributes = AgentActivityAttributes(
            machineId: agent.machineId, terminalId: agent.terminalId, machineLabel: machine.label, nodeId: machine.nodeId,
            title: title
        )
        let activity = try Activity.request(attributes: attributes, content: Self.activityContent(state), pushType: .token)
        watch(activity)
    }

    private func activities(for agent: FollowedAgent) -> [Activity<AgentActivityAttributes>] {
        Activity<AgentActivityAttributes>.activities.filter {
            $0.isLive && !$0.attributes.isOutdated && $0.attributes.machineId == agent.machineId
                && $0.attributes.terminalId == agent.terminalId
        }
    }

    /// Registers every token iOS issues for the activity; once iOS ends it, the Mac forgets it.
    private func watch(_ activity: Activity<AgentActivityAttributes>) {
        guard watchers[activity.id] == nil else { return }
        watchers[activity.id] = Task { [weak self] in
            let tokens = Task {
                for await token in activity.pushTokenUpdates {
                    self?.register(activity, token: token)
                }
            }
            defer { tokens.cancel() }
            for await state in activity.activityStateUpdates where state == .ended || state == .dismissed {
                break
            }
            guard !Task.isCancelled else { return }
            self?.forget(machineId: activity.attributes.machineId, activityId: activity.id)
        }
    }

    private func register(_ activity: Activity<AgentActivityAttributes>, token: Data) {
        if list.remember(RegisteredActivity(machineId: activity.attributes.machineId, activityId: activity.id)) {
            list.save(to: file)
        }
        let hex = token.map { String(format: "%02x", $0) }.joined()
        do {
            try core?.registerActivityToken(
                machineId: activity.attributes.machineId, activityId: activity.id,
                terminalId: activity.attributes.terminalId, tokenHex: hex
            )
        } catch {
            log.error("activity token: \(describe(error), privacy: .public)")
        }
    }

    private func end(_ activity: Activity<AgentActivityAttributes>, dismissal: ActivityUIDismissalPolicy) {
        forget(machineId: activity.attributes.machineId, activityId: activity.id)
        let id = activity.id
        Task.detached { await Self.live(id)?.end(nil, dismissalPolicy: dismissal) }
    }

    /// Activity is not Sendable, so a detached task looks it up again by id.
    private nonisolated static func live(_ id: String) -> Activity<AgentActivityAttributes>? {
        Activity<AgentActivityAttributes>.activities.first { $0.id == id }
    }

    private func forget(machineId: String, activityId: String) {
        watchers.removeValue(forKey: activityId)?.cancel()
        if list.forget(activityId: activityId) {
            list.save(to: file)
        }
        do {
            try core?.endActivity(machineId: machineId, activityId: activityId)
        } catch {
            log.error("activity end: \(describe(error), privacy: .public)")
        }
    }
}

extension Activity {
    var isLive: Bool {
        activityState == .active || activityState == .stale
    }
}

extension AgentActivityAttributes {
    /// Started by a build before `nodeId`: it can show neither the command nor the buttons.
    var isOutdated: Bool { nodeId == nil }
}

extension DecideApprovalIntent {
    /// Set by the AppDelegate at launch, before iOS hands the app any intent.
    @MainActor static var decide: ((ApprovalLink, ApprovalDecision) async -> Void)?

    @MainActor
    func perform() async throws -> some IntentResult {
        guard let link = ApprovalLink(nodeId: nodeId, approvalId: approvalId) else { throw DecideApprovalError.invalidIds }
        // Defense in depth under `authenticationPolicy`: never decide while the device is locked.
        guard UIApplication.shared.isProtectedDataAvailable else { throw DecideApprovalError.locked }
        guard let decide = Self.decide else { throw DecideApprovalError.notReady }
        await decide(link, decision == .approve ? .approve : .deny)
        return .result()
    }
}

enum DecideApprovalError: Error, Equatable {
    case invalidIds, locked, notReady
}

extension AgentActivityStatus {
    init(_ state: AgentState) {
        switch state {
        case .idle: self = .idle
        case .working: self = .working
        case .blocked: self = .blocked
        case .done: self = .done
        case .unknown: self = .unknown
        }
    }
}

extension AgentActivityAttributes.ContentState {
    init(agent: AgentSummary, workspace: String?, approvals: Int) {
        self.init(
            status: AgentActivityStatus(agent.status),
            statusSince: Date(timeIntervalSince1970: TimeInterval(agent.statusSinceMs / 1000)),
            title: agent.alertTitle,
            kind: agent.kind.flatMap { Self.kinds.contains($0) ? $0 : nil },
            workspace: workspace,
            approvals: approvals
        )
    }

    /// The approval fields come only from collied's pushes: a local update keeps them while the
    /// agent is still blocked, and drops them once it is not.
    func keepingApproval(of current: Self) -> Self {
        guard status == .blocked, current.status == .blocked else { return self }
        var state = self
        state.approvalId = current.approvalId
        state.enc = current.enc
        state.progress = current.progress
        return state
    }
}

extension AgentSummary {
    /// What collied puts in the alert and Live Activity title: never the terminal title, which
    /// the pane's program controls and which would reach Apple in clear.
    var alertTitle: String {
        [name, kind].compactMap { $0?.trimmingCharacters(in: .whitespacesAndNewlines) }.first { !$0.isEmpty }
            .map { String($0.prefix(64)) } ?? "agent"
    }

    /// The Live Activity's static title: what the app shows, kept on the phone in the attributes.
    var activityTitle: String { String(displayTitle.prefix(64)) }
}

/// "Follow on Lock Screen" / "Stop following", for the agent screen's More menu and the Agents list.
struct FollowMenuItem: View {
    let follows: FollowModel
    let route: AgentRoute
    @Environment(\.openURL) private var openURL

    var body: some View {
        if follows.isFollowing(route) {
            Button("Stop following", systemImage: "pin.slash") { follows.unfollow(route) }
        } else if follows.enabled {
            Button("Follow on Lock Screen", systemImage: "pin") { follows.follow(route) }
        } else {
            Button("Live Activities are off in Settings", systemImage: "gear") {
                if let url = URL(string: UIApplication.openSettingsURLString) { openURL(url) }
            }
        }
    }
}

extension View {
    func followNotice(_ follows: FollowModel?) -> some View {
        alert(
            "Lock Screen",
            isPresented: Binding(get: { follows?.notice != nil }, set: { if !$0 { follows?.notice = nil } }),
            actions: { Button("OK", role: .cancel) {} },
            message: { Text(follows?.notice ?? "") }
        )
    }
}
