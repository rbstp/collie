import CollieCore
import Foundation
import Network
import Observation
import SwiftUI
import UIKit
import UserNotifications
import os

enum AppTab: CaseIterable, Hashable {
    case agents, machines, settings
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
    private(set) var relayStatus: String?
    @ObservationIgnored private var relay: RelayEnrollment?
    @ObservationIgnored private var relayTask: Task<Void, Never>?
    @ObservationIgnored private var relayDelivered: [String: RelayRouteGrant] = [:]
    @ObservationIgnored private var relayRetryAt = Date.distantPast
    @ObservationIgnored private var relayFailures = 0
    private var alertsOff = false
    let approvals: ApprovalsModel
    let follows: FollowModel
    let watch = WatchLink()
    var tab = AppTab.agents
    var accent = DevicePrefs.load(from: DevicePrefs.file).accent {
        didSet { DevicePrefs.update(in: DevicePrefs.file) { $0.accent = accent } }
    }
    var openingAgent: AgentRoute?
    var approvalOpenError: String?
    @ObservationIgnored private var openingApproval: Task<Void, Never>?
    @ObservationIgnored var viewingAgent: AgentRoute?
    private var backgroundedAt: Date?
    @ObservationIgnored private var activityDecisions: Set<String> = []
    @ObservationIgnored private var terminalKeySet = false
    @ObservationIgnored private let pathMonitor = NWPathMonitor()

    private let log = Logger(subsystem: "dev.rbstp.collie", category: "app")

    init() {
        do {
            #if DEBUG && targetEnvironment(simulator)
            if SimulatorVM.enabled { try SimulatorVM.verifyCore() }
            #endif
            let core = try CollieCore(stateDir: StateDirectory.prepare().path)
            let identity = try Identity.load()
            try core.setIdentity(publicKey: identity.publicKey, signer: identity.signer)
            core.setLog(log: CoreLogRelay())
            if let on = DevicePrefs.load(from: DevicePrefs.file).archiveOnMerge {
                core.setArchiveOnMerge(enabled: on)
            }
            self.core = core
        } catch {
            core = nil
            startupError = describe(error)
        }
        approvals = ApprovalsModel(core: core)
        follows = FollowModel(core: core, approvals: approvals)
        machines = core?.machines() ?? []
        if let marker = try? StateDirectory.prepare().appending(path: "relay-installation"),
            FileManager.default.fileExists(atPath: marker.path)
        {
            do { _ = try relayClient() } catch { relayStatus = "Relay credentials are unavailable" }
        }
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
        #if DEBUG && targetEnvironment(simulator)
        if ProcessInfo.processInfo.environment["COLLIE_SIMULATOR_RELAY"] == "mock" {
            pushToken = Data(repeating: 0xaa, count: 32)
            pushStatus = "Simulator mock destination. No APNs delivery."
        }
        #endif
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
        if UIApplication.shared.applicationState == .active {
            await DoneNotification.removeDelivered()
        }
        guard let core else { return }
        #if DEBUG && targetEnvironment(simulator)
        if SimulatorVM.enabled {
            guard await start(authKey: nil) else { return }
            do {
                let deadline = ContinuousClock.now + .seconds(30)
                repeat {
                    await refreshNode()
                    if isRunning { break }
                    guard ContinuousClock.now < deadline else { throw CocoaError(.featureUnsupported) }
                    try await Task.sleep(for: .milliseconds(200))
                } while true
                machinePaired(try await SimulatorVM.pair(core: core))
            } catch { startupError = describe(error) }
            return
        }
        #endif
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

    /// Shown only on this phone; nil or a blank name restores the name from pairing.
    func renameMachine(_ machine: Machine, to name: String?) throws {
        try core?.renameMachine(machineId: machine.id, name: name)
        reloadMachines()
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
        if let relay {
            try await relay.prepareRevocation(pairingId: machine.id)
            relayStatus = "Revocation pending. Remote pushes may continue until it succeeds or the grant expires."
        }
        PromptResends.shared.stop(machineId: machine.id)
        let unpaired = try await core?.removeMachine(id: machine.id) ?? false
        AgentDrafts.forget(machineId: machine.id, file: AgentDrafts.file)
        SeenAgents.forget(machineId: machine.id, file: SeenAgents.file)
        SnoozedAgents.forget(machineId: machine.id, file: SnoozedAgents.file)
        GitViewed.forget(machineId: machine.id, file: GitViewed.file)
        DevicePrefs.forgetMachine(machineId: machine.id, in: DevicePrefs.file)
        if !(core?.machines() ?? []).contains(where: { $0.nodeId == machine.nodeId }) {
            NotificationKey.delete(nodeId: machine.nodeId)
        }
        relayDelivered.removeValue(forKey: machine.id)
        if let relay {
            Task {
                do { try await relay.retryPendingRevocations(); await updateRelayStatus() }
                catch { relayStatus = "Revocation pending. Remote pushes may continue until it succeeds or the grant expires." }
            }
        }
        return unpaired
    }

    /// A pairing or re-pairing rotates that Mac's notification key.
    func machinePaired(_ machine: Machine) {
        NotificationKey.delete(nodeId: machine.nodeId)
        reloadMachines()
        if let relay {
            Task {
                do {
                    try await relay.confirmNewPairing()
                    if let pushToken { registerPush(token: pushToken) }
                } catch { relayStatus = "Relay credentials are unavailable" }
            }
        } else if let pushToken { registerPush(token: pushToken) }
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
            Task { await DoneNotification.removeDelivered() }
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
        relayDelivered.removeAll()
        relayRetryAt = .distantPast
        guard let core else { return }
        let environment = PushEnvironment.current
        let hex = token.hexString
        let muteDone = !DevicePrefs.load(from: DevicePrefs.file).doneAlerts
        var failure: (any Error)?
        for machine in core.machines() {
            if core.pushMode(machineId: machine.id) == .relay { continue }
            do {
                let key = try NotificationKey.loadOrCreate(nodeId: machine.nodeId)
                try core.registerPush(
                    machineId: machine.id, apnsTokenHex: hex, environment: environment,
                    notificationKey: key.withUnsafeBytes { Data($0) }, muteDone: muteDone, relayGrant: nil
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
        refreshRelayRegistration()
    }

    private func relayClient() throws -> RelayEnrollment {
        if let relay { return relay }
        #if DEBUG && targetEnvironment(simulator)
        if let value = try SimulatorRelay.make() {
            relay = value
            return value
        }
        #endif
        let store = RelayCredentialStore(marker: try StateDirectory.prepare().appending(path: "relay-installation"))
        let value = RelayEnrollment(state: try store.load(), environment: PushEnvironment.current == .production ? "production" : "development",
                                    persist: { try store.save($0) })
        relay = value
        return value
    }

    private func updateRelayStatus() async {
        guard let relay else { return }
        let state = await relay.snapshot()
        if !state.pendingPairingRevocations.isEmpty || state.pendingInstallationRevocation {
            relayStatus = "Revocation pending. Remote pushes may continue until it succeeds or the grant expires."
        } else if state.requiresPairing {
            relayStatus = "Relay key unavailable. Pair the machine again to restore notifications. Previous grants may continue until expiry."
        } else if let previous = state.previousGrantExpiry, previous > UInt64(Date().timeIntervalSince1970) {
            relayStatus = "Previous relay grants may continue until \(Date(timeIntervalSince1970: TimeInterval(previous)).formatted(date: .abbreviated, time: .omitted))."
        } else if let expiry = state.routes.map(\.grant.expiresAt).min() {
            relayStatus = "Relay notifications expire \(Date(timeIntervalSince1970: TimeInterval(expiry)).formatted(date: .abbreviated, time: .omitted)). Open Collie to renew before expiry."
        }
    }

    func refreshRelayRegistration() {
        guard relayTask == nil, Date.now >= relayRetryAt, let core, let pushToken else { return }
        let eligible = core.machines().filter { core.pushMode(machineId: $0.id) == .relay }
        guard !eligible.isEmpty || relay != nil else { return }
        relayTask = Task {
            defer { relayTask = nil }
            do {
                let client = try relayClient()
                try await client.retryPendingRevocations()
                for machine in eligible {
                    let grant = try await client.register(pairingId: machine.id, destination: pushToken.hexString)
                    guard core.machines().contains(where: { $0.id == machine.id }), core.pushMode(machineId: machine.id) == .relay else { continue }
                    if relayDelivered[machine.id] != grant {
                        let key = try NotificationKey.loadOrCreate(nodeId: machine.nodeId)
                        try core.registerPush(machineId: machine.id, apnsTokenHex: pushToken.hexString, environment: .current,
                                              notificationKey: key.withUnsafeBytes { Data($0) },
                                              muteDone: !DevicePrefs.load(from: DevicePrefs.file).doneAlerts, relayGrant: grant.coreGrant)
                        relayDelivered[machine.id] = grant
                    }
                }
                follows.relayReady(client)
                relayFailures = 0
                await updateRelayStatus()
            } catch {
                relayFailures += 1
                relayRetryAt = .now.addingTimeInterval(min(600, pow(2, Double(min(relayFailures, 7))) * 5))
                relayStatus = "Relay notifications are unavailable. Paired sessions remain usable."
                if let state = await relay?.snapshot(), state.requiresPairing || !state.pendingPairingRevocations.isEmpty || state.pendingInstallationRevocation {
                    await updateRelayStatus()
                }
            }
        }
    }

    /// Reaches each connected Mac now and the others on their next connection.
    func setDoneAlerts(_ on: Bool) {
        DevicePrefs.update(in: DevicePrefs.file) { $0.doneAlerts = on }
        if let pushToken { registerPush(token: pushToken) }
    }

    func setArchiveOnMerge(_ on: Bool) {
        DevicePrefs.update(in: DevicePrefs.file) { $0.archiveOnMerge = on }
        core?.setArchiveOnMerge(enabled: on)
    }

    func pushRegistrationFailed(_ error: any Error) {
        pushStatus = describe(error)
    }

    func open(_ link: ApprovalLink, terminalId: String? = nil) {
        cancelApprovalOpening()
        tab = .agents
        let pending = approvals.open(link)
        openingApproval = Task { [weak self] in
            await pending?.value
            guard !Task.isCancelled, let self else { return }
            reloadMachines()
            if let route = approvals.session(for: link) ?? Self.route(for: link, terminalId: terminalId, machines: machines),
               machines.contains(where: { $0.id == route.machineId }) {
                openingAgent = route
            } else {
                let message = approvals.notice ?? "This approval is no longer pending."
                approvalOpenError = message
                log.error("approval open: \(message, privacy: .public)")
            }
        }
    }

    func cancelApprovalOpening() {
        openingApproval?.cancel()
        openingApproval = nil
        approvals.cancelOpening()
        approvalOpenError = nil
    }

    /// A Live Activity tap: the Agents tab, on that agent. Links to a Mac that is not paired are ignored.
    func open(_ url: URL) {
        guard let route = Self.route(for: url, machines: machines) else { return }
        cancelApprovalOpening()
        tab = .agents
        openingAgent = route
    }

    /// A done alert for the agent on screen shows no banner or sound.
    func isViewing(nodeId: String, terminalId: String) -> Bool {
        guard tab == .agents, let viewingAgent, let machine = machines.first(where: { $0.nodeId == nodeId }) else { return false }
        return viewingAgent == AgentRoute(machineId: machine.id, terminalId: terminalId)
    }

    nonisolated static func route(for url: URL, machines: [Machine]) -> AgentRoute? {
        guard let link = AgentLink(url: url), machines.contains(where: { $0.id == link.machineId }) else { return nil }
        return AgentRoute(machineId: link.machineId, terminalId: link.terminalId)
    }

    nonisolated static func route(for link: ApprovalLink, terminalId: String?, machines: [Machine]) -> AgentRoute? {
        guard let terminalId, let machine = machines.first(where: { $0.nodeId == link.nodeId }),
              AgentLink(machineId: machine.id, terminalId: terminalId) != nil else { return nil }
        return AgentRoute(machineId: machine.id, terminalId: terminalId)
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

extension Data {
    var hexString: String { map { String(format: "%02x", $0) }.joined() }
}

final class CoreLogRelay: CoreLog {
    private let logger = Logger(subsystem: "dev.rbstp.collie", category: "core")

    func log(message: String) {
        logger.notice("\(message, privacy: .public)")
    }
}


extension RelayRouteGrant {
    var coreGrant: RelayPushGrant {
        RelayPushGrant(token: token, routeId: routeId, generation: generation, expiresAt: expiresAt)
    }
}
