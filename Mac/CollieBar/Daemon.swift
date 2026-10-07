import AppKit
import Foundation
import Observation
import ServiceManagement

@MainActor
@Observable
final class Daemon {
    private(set) var state: DaemonState = .off
    private(set) var starting = false
    private(set) var busy = false
    private(set) var peers: [Peer] = []
    private(set) var lastError: String?
    private(set) var auditLogExists = false
    private(set) var openAtLogin = false

    let socketPath: String
    private let dataDir: URL
    private let plist: URL

    init() {
        // HOME as collied reads it (config::home_dir), so a test collied with its own HOME works.
        let home = URL(filePath: ProcessInfo.processInfo.environment["HOME"] ?? NSHomeDirectory())
        dataDir = home.appending(path: "Library/Application Support/collie")
        socketPath = dataDir.appending(path: "control.sock").path(percentEncoded: false)
        plist = home.appending(path: "Library/LaunchAgents/dev.rbstp.collied.plist")
        let socket = socketPath
        Task {
            await watchDaemon(socket: socket, retry: .seconds(2)) { [weak self] state in
                await self?.apply(state)
            }
        }
        _ = NotificationCenter.default.addObserver(
            forName: NSMenu.didBeginTrackingNotification, object: nil, queue: .main
        ) { [weak self] _ in
            MainActor.assumeIsolated { self?.menuOpened() }
        }
    }

    var running: Bool { state != .off }

    var pendingApprovals: Int {
        if case .running(let n) = state { n } else { 0 }
    }

    private func apply(_ new: DaemonState) {
        if new != .off { starting = false }
        if state != new { state = new }
        if new == .off, !peers.isEmpty { peers = [] }
    }

    private func menuOpened() {
        auditLogExists = FileManager.default.fileExists(atPath: auditLog.path(percentEncoded: false))
        openAtLogin = SMAppService.mainApp.status == .enabled
        refreshPeers()
    }

    func refreshPeers() {
        guard running else { return }
        let socket = socketPath
        Task {
            if case .peers(let list) = try? await request(.peersList, socket: socket) {
                peers = list
            }
        }
    }

    func toggle() {
        let on = !running
        Task {
            guard await collied(on ? "start" : "stop") else { return }
            guard on, !running else { return }
            starting = true
            // The tailnet can take up to a minute to come up.
            try? await Task.sleep(for: .seconds(90))
            starting = false
        }
    }

    func quit() {
        Task {
            _ = await collied("stop")
            NSApp.terminate(nil)
        }
    }

    private var auditLog: URL { dataDir.appending(path: "audit.log") }

    func openAuditLog() {
        NSWorkspace.shared.open(auditLog)
    }

    func setOpenAtLogin(_ on: Bool) {
        let service = SMAppService.mainApp
        do {
            if on { try service.register() } else { try service.unregister() }
            lastError = nil
        } catch {
            lastError = "Open at Login: \(error.localizedDescription)"
        }
        if service.status == .requiresApproval { SMAppService.openSystemSettingsLoginItems() }
        openAtLogin = service.status == .enabled
    }

    /// `collied start|stop` from the installed service's plist: launchd keeps the semantics
    /// (disabled until start, across reboots) in one place, and the app runs nothing else.
    private func collied(_ command: String) async -> Bool {
        busy = true
        defer { busy = false }
        guard let data = try? Data(contentsOf: plist),
            let dict = try? PropertyListSerialization.propertyList(from: data, format: nil) as? [String: Any],
            let args = dict["ProgramArguments"] as? [String],
            let exe = args.first
        else {
            lastError = "not installed: run collied service install"
            return false
        }
        let result = await run(exe, [command])
        // "already stopped" from `collied stop` means a collied launchd does not manage.
        lastError = result.ok && !result.output.hasPrefix("already") ? nil : printable(result.output)
        return result.ok
    }

    private nonisolated func run(_ exe: String, _ args: [String]) async -> (ok: Bool, output: String) {
        let process = Process()
        process.executableURL = URL(filePath: exe)
        process.arguments = args
        let pipe = Pipe()
        process.standardOutput = pipe
        process.standardError = pipe
        return await withCheckedContinuation { continuation in
            process.terminationHandler = { p in
                let out = String(decoding: pipe.fileHandleForReading.readDataToEndOfFile(), as: UTF8.self)
                continuation.resume(
                    returning: (p.terminationStatus == 0, out.trimmingCharacters(in: .whitespacesAndNewlines)))
            }
            do {
                try process.run()
            } catch {
                continuation.resume(returning: (false, "cannot run \(exe)"))
            }
        }
    }
}
