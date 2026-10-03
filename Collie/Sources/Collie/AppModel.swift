import CollieCore
import Foundation
import Observation
import SwiftUI
import os

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
    private var backgroundedAt: Date?

    private let log = Logger(subsystem: "dev.rbstp.collie", category: "app")

    init() {
        do {
            core = try CollieCore(stateDir: StateDirectory.prepare().path)
        } catch {
            core = nil
            startupError = describe(error)
        }
        machines = core?.machines() ?? []
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

func describe(_ error: any Error) -> String {
    if let error = error as? CoreError {
        return error.description
    }
    return error.localizedDescription
}
