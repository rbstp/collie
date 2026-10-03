import CollieCore
import Foundation
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
    private(set) var signingIn = false
    private(set) var pushStatus = "not registered"
    let approvals: ApprovalsModel
    var tab = AppTab.agents
    private var backgroundedAt: Date?

    private let log = Logger(subsystem: "dev.rbstp.collie", category: "app")

    init() {
        do {
            core = try CollieCore(stateDir: StateDirectory.prepare().path)
        } catch {
            core = nil
            startupError = describe(error)
        }
        approvals = ApprovalsModel(core: core)
        machines = core?.machines() ?? []
        if let core, let group = AppGroup.container {
            do {
                try core.setAppGroupDir(path: group.path)
            } catch {
                log.error("app group: \(describe(error), privacy: .public)")
            }
        }
    }

    var isRunning: Bool { node?.backendState == .running }

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
        machines = core?.machines() ?? []
    }

    func removeMachine(_ machine: Machine) throws {
        try core?.removeMachine(id: machine.id)
        reloadMachines()
    }

    func scenePhaseChanged(to phase: ScenePhase) {
        switch phase {
        case .background:
            backgroundedAt = .now
        case .active:
            if let since = backgroundedAt {
                core?.resume(backgroundSecs: UInt64(max(0, Date.now.timeIntervalSince(since))))
            }
            backgroundedAt = nil
        default:
            break
        }
    }

    /// Asks once, after onboarding; later launches only refresh the APNs token.
    func enableNotifications() async {
        let granted = (try? await UNUserNotificationCenter.current().requestAuthorization(options: [.alert, .sound, .badge])) ?? false
        if granted {
            UIApplication.shared.registerForRemoteNotifications()
        } else {
            pushStatus = "notifications are off for collie"
        }
    }

    func registerPush(token: Data) {
        let environment = PushEnvironment.current
        do {
            try core?.registerPush(apnsTokenHex: token.map { String(format: "%02x", $0) }.joined(), environment: environment)
            pushStatus = environment == .production ? "registered" : "registered (sandbox)"
        } catch {
            pushStatus = describe(error)
        }
    }

    func pushRegistrationFailed(_ error: any Error) {
        pushStatus = describe(error)
    }

    func open(_ link: ApprovalLink) {
        tab = .approvals
        approvals.open(link)
    }

    /// Lock-screen Approve/Deny. iOS has already required the device owner to unlock
    /// (the actions are `authenticationRequired`); the app may have been launched in the
    /// background for this alone.
    func decideFromNotification(_ link: ApprovalLink, _ decision: ApprovalDecision, agent: String, thread: String) async {
        let assertion = BackgroundAssertion(name: "approval.decide")
        defer { assertion.end() }
        let followUp: FollowUp
        if let core {
            let report = await core.decideFromNotification(
                machineNodeId: link.nodeId, approvalId: link.approvalId, decision: decision, budgetMs: 20_000
            )
            log.notice(
                "lock-screen decide: outcome=\(String(describing: report.outcome), privacy: .public) nodeWasRunning=\(report.nodeWasRunning, privacy: .public) nodeUp=\(report.nodeUpMs.map(String.init) ?? "-", privacy: .public)ms connect=\(report.connectMs.map(String.init) ?? "-", privacy: .public)ms lookup=\(report.lookupMs.map(String.init) ?? "-", privacy: .public)ms decide=\(report.decideMs.map(String.init) ?? "-", privacy: .public)ms total=\(report.totalMs, privacy: .public)ms"
            )
            followUp = FollowUp.after(report.outcome, decision: decision, agent: agent)
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
