import CollieCore
import Foundation
import Network
import Observation
import SwiftUI
import UIKit
import UserNotifications
import os

enum AppTab: Hashable {
    case agents, approvals, machines, settings
}

@MainActor
@Observable
final class AppModel {
    let core: CollieCore?
    private(set) var startupError: String?
    private(set) var node: NodeState?
    private(set) var nodeError: String?
    private(set) var signInError: String?
    private(set) var machines: [Machine] = []
    private var removing: Set<String> = []
    private(set) var signingIn = false
    private(set) var pushStatus = "not registered"
    private var pushToken: Data?
    private var alertsOff = false
    let approvals: ApprovalsModel
    let follows: FollowModel
    let watch = WatchLink()
    var tab = AppTab.agents
    var openingAgent: AgentRoute?
    private var backgroundedAt: Date?
    @ObservationIgnored private var activityDecisions: Set<String> = []
    @ObservationIgnored private var terminalKeySet = false
    @ObservationIgnored private let pathMonitor = NWPathMonitor()

    private let log = Logger(subsystem: "dev.rbstp.collie", category: "app")

    init() {
        do {
            let core = try CollieCore(stateDir: StateDirectory.prepare().path)
            let identity = try Identity.load()
            try core.setIdentity(publicKey: identity.publicKey, signer: identity.signer)
            core.setLog(log: CoreLogRelay())
            self.core = core
        } catch {
            core = nil
            startupError = describe(error)
        }
        approvals = ApprovalsModel(core: core)
        follows = FollowModel(core: core, approvals: approvals)
        machines = core?.machines() ?? []
        if let core, let group = AppGroup.container {
            do {
                try core.setAppGroupDir(path: group.path)
            } catch {
                log.error("app group: \(describe(error), privacy: .public)")
            }
        }
        if let core {
            pathMonitor.pathUpdateHandler = { path in
                core.setLowData(lowData: path.isConstrained)
                core.networkChanged(interface: Self.interface(of: path))
            }
            pathMonitor.start(queue: DispatchQueue(label: "dev.rbstp.collie.path"))
        }
        loadTerminalKey()
    }

    /// The interface iOS routes through, never a VPN tunnel; empty when there is no path.
    nonisolated static func interface(of path: NWPath) -> String {
        guard path.status == .satisfied else { return "" }
        return path.availableInterfaces.first { [.wifi, .cellular, .wiredEthernet].contains($0.type) }?.name ?? ""
    }

    var isRunning: Bool { node?.backendState == .running }

    /// A phone that joined the tailnet before goes straight to the tabs while its node starts;
    /// onboarding only comes back when Tailscale asks for a sign-in again.
    var showsMain: Bool {
        if isRunning { return true }
        guard core?.tailnetConfigured() == true else { return false }
        switch node?.backendState {
        case .needsLogin, .needsMachineAuth: return false
        default: return true
        }
    }

    func launch() async {
        guard let core else { return }
        if core.tailnetConfigured() || ProcessInfo.processInfo.arguments.contains("--measure-cold-start") {
            await start(authKey: nil)
        }
        await refreshNode()
        if ProcessInfo.processInfo.arguments.contains("--measure-cold-start") {
            await logColdStart()
        }
    }

    func refreshNode() async {
        guard let core else { return }
        do {
            node = try await core.nodeState()
            nodeError = nil
        } catch {
            nodeError = describe(error)
        }
    }

    func signIn(openLogin: (URL) -> Void, closeLogin: () -> Void) async {
        guard await start(authKey: nil) else { return }
        signingIn = true
        defer {
            signingIn = false
            closeLogin()
        }
        var opened = false
        while !Task.isCancelled {
            await refreshNode()
            if isRunning { return }
            if !opened, let url = node?.authUrl.flatMap(URL.init(string:)), url.scheme == "https" {
                opened = true
                openLogin(url)
            }
            try? await Task.sleep(for: .milliseconds(500))
        }
    }

    func signIn(authKey: String) async {
        guard await start(authKey: authKey) else { return }
        signingIn = true
        defer { signingIn = false }
        let deadline = ContinuousClock.now + .seconds(30)
        while !Task.isCancelled {
            await refreshNode()
            if isRunning || node?.backendState == .needsMachineAuth { return }
            if ContinuousClock.now >= deadline {
                signInError = "Tailscale did not accept the auth key. Check that it is valid and not expired, then try again."
                return
            }
            try? await Task.sleep(for: .milliseconds(500))
        }
    }

    @discardableResult
    private func start(authKey: String?) async -> Bool {
        guard let core else { return false }
        signInError = nil
        do {
            try await core.nodeStart(authKey: authKey)
            return true
        } catch {
            signInError = describe(error)
            return false
        }
    }

    func reloadMachines() {
        machines = (core?.machines() ?? []).filter { !removing.contains($0.id) }
    }

    /// The row leaves the list now; removeMachine finishes the removal.
    func hideMachine(_ machine: Machine) {
        removing.insert(machine.id)
        reloadMachines()
    }

    /// False when the machine did not confirm it removed this phone.
    func removeMachine(_ machine: Machine) async throws -> Bool {
        hideMachine(machine)
        defer {
            removing.remove(machine.id)
            reloadMachines()
        }
        let unpaired = try await core?.removeMachine(id: machine.id) ?? false
        AgentDrafts.forget(machineId: machine.id, file: AgentDrafts.file)
        DevicePrefs.forgetTaskBase(machineId: machine.id, in: DevicePrefs.file)
        if !(core?.machines() ?? []).contains(where: { $0.nodeId == machine.nodeId }) {
            NotificationKey.delete(nodeId: machine.nodeId)
        }
        return unpaired
    }

    /// A pairing or re-pairing rotates that Mac's notification key.
    func machinePaired(_ machine: Machine) {
        NotificationKey.delete(nodeId: machine.nodeId)
        reloadMachines()
        if let pushToken { registerPush(token: pushToken) }
    }

    /// The key is in the Keychain only while the phone is unlocked, so a launch in the background
    /// (a lock-screen decision) loads it on the next foreground.
    private func loadTerminalKey() {
        guard let core, !terminalKeySet, let key = TerminalKey.publicKey() else { return }
        do {
            try core.setTerminalKey(publicKey: key)
            terminalKeySet = true
        } catch {
            log.error("terminal key: \(describe(error), privacy: .public)")
        }
    }

    func scenePhaseChanged(to phase: ScenePhase) {
        switch phase {
        case .background:
            backgroundedAt = .now
            publishWatchState(leaving: true)
            if let core {
                let assertion = BackgroundAssertion(name: "core.suspend")
                let epoch = core.beginSuspend()
                Task {
                    // Not on .inactive: the Face ID sheet itself makes the app inactive.
                    await core.lockTerminals()
                    // Follow then lock: the activity's token must reach its Mac before the sessions close.
                    let deadline = ContinuousClock.now + .seconds(3)
                    while follows.awaitingToken, ContinuousClock.now < deadline {
                        try? await Task.sleep(for: .milliseconds(100))
                    }
                    await core.suspend(epoch: epoch)
                    assertion.end()
                }
            }
        case .active:
            loadTerminalKey()
            Task { await refreshNode() }
            if let since = backgroundedAt {
                core?.resume(backgroundSecs: UInt64(max(0, Date.now.timeIntervalSince(since))))
            }
            backgroundedAt = nil
            follows.foreground()
        default:
            break
        }
    }

    /// Asks once, after onboarding; later launches only refresh the APNs token. Registers even
    /// when alerts are denied: collied refuses Live Activity tokens from a device without `push.register`.
    func enableNotifications() async {
        let granted = (try? await UNUserNotificationCenter.current().requestAuthorization(options: [.alert, .sound, .badge])) ?? false
        alertsOff = !granted
        if alertsOff { pushStatus = "notifications are off for collie" }
        UIApplication.shared.registerForRemoteNotifications()
    }

    /// One notification key per Mac, keyed by its node id: the NSE only sees `node_id` in the alert.
    func registerPush(token: Data) {
        pushToken = token
        guard let core else { return }
        let environment = PushEnvironment.current
        let hex = token.map { String(format: "%02x", $0) }.joined()
        let muteDone = !DevicePrefs.load(from: DevicePrefs.file).doneAlerts
        var failure: (any Error)?
        for machine in core.machines() {
            do {
                let key = try NotificationKey.loadOrCreate(nodeId: machine.nodeId)
                try core.registerPush(
                    machineId: machine.id, apnsTokenHex: hex, environment: environment,
                    notificationKey: key.withUnsafeBytes { Data($0) }, muteDone: muteDone
                )
            } catch {
                failure = failure ?? error
            }
        }
        if let failure {
            pushStatus = describe(failure)
        } else if alertsOff {
            pushStatus = "registered, notifications are off for collie"
        } else {
            pushStatus = environment == .production ? "registered" : "registered (sandbox)"
        }
    }

    /// Reaches each connected Mac now and the others on their next connection.
    func setDoneAlerts(_ on: Bool) {
        DevicePrefs.update(in: DevicePrefs.file) { $0.doneAlerts = on }
        if let pushToken { registerPush(token: pushToken) }
    }

    func pushRegistrationFailed(_ error: any Error) {
        pushStatus = describe(error)
    }

    func open(_ link: ApprovalLink) {
        tab = .approvals
        approvals.open(link)
    }

    /// A Live Activity tap: the Agents tab, on that agent. Links to a Mac that is not paired are ignored.
    func open(_ url: URL) {
        guard let route = Self.route(for: url, machines: machines) else { return }
        tab = .agents
        openingAgent = route
    }

    nonisolated static func route(for url: URL, machines: [Machine]) -> AgentRoute? {
        guard let link = AgentLink(url: url), machines.contains(where: { $0.id == link.machineId }) else { return nil }
        return AgentRoute(machineId: link.machineId, terminalId: link.terminalId)
    }

    /// Lock-screen Approve/Deny. iOS has already required the device owner to unlock
    /// (the actions are `authenticationRequired`); the app may have been launched in the
    /// background for this alone. `quietWhenApplied` skips the follow-up when the outcome
    /// already shows elsewhere (on the Live Activity).
    @discardableResult
    func decideFromNotification(
        _ link: ApprovalLink, _ decision: ApprovalDecision, agent: String, thread: String, quietWhenApplied: Bool = false
    ) async -> BackgroundOutcome? {
        let assertion = BackgroundAssertion(name: "approval.decide")
        defer { assertion.end() }
        let followUp: FollowUp
        var outcome: BackgroundOutcome?
        if let core {
            let report = await core.decideFromNotification(
                machineNodeId: link.nodeId, approvalId: link.approvalId, decision: decision, budgetMs: 20_000
            )
            log.notice(
                "lock-screen decide: outcome=\(String(describing: report.outcome), privacy: .public) nodeWasRunning=\(report.nodeWasRunning, privacy: .public) nodeUp=\(report.nodeUpMs.map(String.init) ?? "-", privacy: .public)ms connect=\(report.connectMs.map(String.init) ?? "-", privacy: .public)ms lookup=\(report.lookupMs.map(String.init) ?? "-", privacy: .public)ms decide=\(report.decideMs.map(String.init) ?? "-", privacy: .public)ms total=\(report.totalMs, privacy: .public)ms"
            )
            outcome = report.outcome
            if quietWhenApplied, case .applied = report.outcome { return outcome }
            let machines = core.machines()
            let machine = machines.count > 1 ? machines.first { $0.nodeId == link.nodeId }?.label : nil
            followUp = FollowUp.after(report.outcome, decision: decision, agent: [agent, machine].compactMap { $0 }.joined(separator: " · "))
        } else {
            followUp = FollowUp(title: agent, body: FollowUp.unreachable, opensApproval: true)
        }
        let content = UNMutableNotificationContent()
        content.title = followUp.title
        content.body = followUp.body
        content.threadIdentifier = thread
        if followUp.opensApproval {
            content.userInfo = link.userInfo
            content.sound = .default
        }
        let request = UNNotificationRequest(identifier: "followup-\(link.approvalId)", content: content, trigger: nil)
        do {
            try await UNUserNotificationCenter.current().add(request)
        } catch {
            log.error("follow-up notification: \(describe(error), privacy: .public)")
        }
        return outcome
    }

    /// Approve/Deny on a followed agent's Live Activity, with the same unlock requirement
    /// (`DecideApprovalIntent.authenticationPolicy`) and path as the notification actions.
    func decideFromActivity(_ link: ApprovalLink, _ decision: ApprovalDecision) async {
        // A second tap would only fail as already resolved and post a misleading follow-up.
        guard activityDecisions.insert(link.approvalId).inserted else { return }
        defer { activityDecisions.remove(link.approvalId) }
        let tappedAt = Date.now
        let agent = await FollowModel.show(progress: decision.progressive, on: link)
        let outcome = await decideFromNotification(
            link, decision, agent: agent?.title ?? "agent", thread: agent?.terminalId ?? "", quietWhenApplied: true
        )
        if case .applied = outcome {
            await FollowModel.resolve(link, approved: decision != .deny, tappedAt: tappedAt)
        } else {
            await FollowModel.show(progress: nil, on: link)
        }
    }

    /// Never in the background, where only `refreshForWatch` lists approvals, except `leaving` as
    /// the app goes there; nor before each machine has a snapshot or is known down: an empty list
    /// would clear the watch.
    func publishWatchState(leaving: Bool = false) {
        guard let core, watch.ready, leaving || UIApplication.shared.applicationState != .background else { return }
        let entries = core.machines().map { MachineFlockEntry(machine: $0, flock: core.cachedFlock(machineId: $0.id)) }
        guard entries.allSatisfy({ $0.flock?.details != nil || $0.linkDown }) else { return }
        watch.publish(
            WatchState.published(
                WatchLink.shown(), items: approvals.items, entries: entries,
                allowed: DevicePrefs.load(from: DevicePrefs.file).watchDecisions, live: !leaving, now: .now
            )
        )
    }

    /// From the paired watch as it opens. Out of the foreground (locked, suspended or not
    /// running), the approvals are listed from each Mac over the lock-screen path and sent as
    /// the state the watch may decide on, with the plan usage they carry; the agents stay as last shown.
    func refreshForWatch() async -> (WatchState, silent: [String])? {
        guard let core, watch.ready else { return nil }
        if UIApplication.shared.applicationState == .active {
            publishWatchState()
            return WatchLink.shown().map { ($0, []) }
        }
        let assertion = BackgroundAssertion(name: "watch.refresh")
        defer { assertion.end() }
        let listed = await core.approvalsInBackground(budgetMs: 15_000)
        let machines = core.machines()
        let state = WatchState.refreshed(
            WatchLink.shown(), listed: listed, machines: machines,
            allowed: DevicePrefs.load(from: DevicePrefs.file).watchDecisions, now: .now
        )
        let silent = listed.filter { $0.approvals == nil }.compactMap { entry in machines.first { $0.id == entry.machineId }?.nodeId }
        let counts = listed.map { $0.approvals.map { String($0.count) } ?? "none" }.joined(separator: ",")
        WatchLink.log.notice("refresh: listed=\(counts, privacy: .public) sent=\(state.approvals.count, privacy: .public)")
        guard watch.send(state) else { return nil }
        return (state, silent)
    }

    /// From the paired watch: its wrist check stands in for Face ID only while this phone's
    /// setting allows it, re-read here on every request, and only for an answer this phone sent
    /// to the watch.
    func decideFromWatch(_ request: WatchDecisionRequest) async -> (FollowUp, answered: Bool) {
        let shown = WatchLink.shown()
        let agent = shown?.approvals.first { $0.approvalId == request.approvalId }?.agent ?? "collie"
        let allowed = DevicePrefs.load(from: DevicePrefs.file).watchDecisions
        if let refusal = WatchLink.refusal(request, allowed: allowed, shown: shown, now: .now) {
            return (FollowUp(title: agent, body: refusal, opensApproval: false), false)
        }
        guard let core else { return (FollowUp(title: agent, body: FollowUp.unreachable, opensApproval: true), false) }
        guard activityDecisions.insert(request.approvalId).inserted else {
            return (FollowUp(title: agent, body: "Already sending.", opensApproval: false), false)
        }
        defer { activityDecisions.remove(request.approvalId) }
        let assertion = BackgroundAssertion(name: "watch.decide")
        defer { assertion.end() }
        let decision = request.decision.core
        let report = await core.decideFromNotification(
            machineNodeId: request.nodeId, approvalId: request.approvalId, decision: decision, budgetMs: 20_000
        )
        log.notice("watch decide: outcome=\(String(describing: report.outcome), privacy: .public) total=\(report.totalMs, privacy: .public)ms")
        return (FollowUp.after(report.outcome, decision: decision, agent: agent), report.outcome.watchAnswered)
    }

    private func logColdStart() async {
        for _ in 0..<40 {
            if let report = core?.coldStartReport() {
                log.notice(
                    "cold start: created=\(report.nodeCreatedMs, privacy: .public)ms started=\(report.startedMs, privacy: .public)ms settled=\(report.settledMs.map(String.init) ?? "timeout", privacy: .public)ms state=\(String(describing: report.backendState), privacy: .public) authURL=\(report.authUrlPresent, privacy: .public) polls=\(report.statusPolls, privacy: .public)"
                )
                return
            }
            try? await Task.sleep(for: .milliseconds(500))
        }
    }
}

@MainActor
private final class BackgroundAssertion {
    private var id = UIBackgroundTaskIdentifier.invalid

    init(name: String) {
        id = UIApplication.shared.beginBackgroundTask(withName: name) { [weak self] in self?.end() }
    }

    func end() {
        guard id != .invalid else { return }
        UIApplication.shared.endBackgroundTask(id)
        id = .invalid
    }
}

func describe(_ error: any Error) -> String {
    if let error = error as? CoreError {
        return error.description
    }
    return error.localizedDescription
}

final class CoreLogRelay: CoreLog {
    private let logger = Logger(subsystem: "dev.rbstp.collie", category: "core")

    func log(message: String) {
        logger.notice("\(message, privacy: .public)")
    }
}
