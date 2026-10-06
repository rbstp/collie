import CollieCore
import Foundation
import Observation

struct MachineFlockEntry: Identifiable, Equatable {
    let machine: Machine
    var flock: MachineFlock?
    var error: String?

    var id: String { machine.id }

    var agents: [AgentSummary] { FlockOrder.sorted(flock?.agents ?? []) }

    /// Shell panes, only from a machine that enables terminals.
    var terminals: [TerminalSummary] { flock?.terminalsEnabled == true ? flock?.terminals ?? [] : [] }

    var linkDown: Bool { flock.map { ![.connected, .connecting].contains($0.link) } ?? false }

    func workspaceLabel(for agent: AgentSummary) -> String? {
        flock?.workspaces.first { $0.workspaceId == agent.workspaceId }?.label
    }
}

/// The slice of CollieCore the Agents list uses, so it can run against a fake.
protocol FlockCore: AgentCore {
    func machines() -> [Machine]
    func cachedFlock(machineId: String) -> MachineFlock?
    func reconnect(machineId: String) throws
}

extension CollieCore: FlockCore {}

@MainActor
@Observable
final class FlockModel {
    private(set) var entries: [MachineFlockEntry] = []
    private(set) var refreshing = false

    var close = CloseConfirmation()
    private(set) var closing: AgentRoute?
    private(set) var closeNotice: String?

    func beginClose(_ target: CloseTarget, route: AgentRoute) {
        closing = route
        close.begin(target)
    }

    /// Only reachable after both confirmation steps, as on the agent screen.
    func performClose(core: any AgentCore) async -> Bool {
        guard let route = closing, let target = close.confirm() else { return false }
        do {
            try await core.closeConfirmed(target, route: route)
            closeNotice = nil
            return true
        } catch {
            let message = AgentModel.message(for: error)
            let label = entries.first { $0.id == route.machineId }?.machine.label
            closeNotice = label.map { "\($0): \(message)" } ?? message
            return false
        }
    }

    /// Without `snapshot`, a machine is read from the core's cache, unless its last read failed.
    func refresh(core: (any FlockCore)?, snapshot: Bool = true) async {
        guard let core, !refreshing else { return }
        refreshing = true
        defer { refreshing = false }
        let machines = core.machines()
        let seeded = machines.map { machine in
            entries.first { $0.id == machine.id }.map { MachineFlockEntry(machine: machine, flock: $0.flock, error: $0.error) }
                ?? MachineFlockEntry(machine: machine)
        }
        if seeded != entries { entries = seeded }
        await withTaskGroup(of: MachineFlockEntry.self) { group in
            for entry in seeded {
                let machine = entry.machine
                let cached = snapshot || entry.error != nil ? nil : core.cachedFlock(machineId: machine.id)
                group.addTask {
                    do {
                        let flock: MachineFlock
                        if let cached { flock = cached } else { flock = try await core.flock(machineId: machine.id) }
                        return MachineFlockEntry(machine: machine, flock: flock, error: flock.link == .connecting ? nil : flock.lastError)
                    } catch {
                        return MachineFlockEntry(
                            machine: machine,
                            flock: core.cachedFlock(machineId: machine.id),
                            error: describe(error)
                        )
                    }
                }
            }
            for await entry in group {
                if let index = entries.firstIndex(where: { $0.id == entry.id }), entries[index] != entry {
                    entries[index] = entry
                }
            }
        }
    }
}

enum FlockOrder {
    static func rank(_ state: AgentState) -> Int {
        switch state {
        case .blocked: 0
        case .working: 1
        case .idle: 2
        case .done: 3
        case .unknown: 4
        }
    }

    static func sorted(_ agents: [AgentSummary]) -> [AgentSummary] {
        agents.sorted {
            (rank($0.status), $0.statusSinceMs, $0.terminalId) < (rank($1.status), $1.statusSinceMs, $1.terminalId)
        }
    }

    static func neighbor(of route: AgentRoute, offset: Int, in entries: [MachineFlockEntry]) -> AgentRoute? {
        let routes = entries.flatMap { entry in entry.agents.map { AgentRoute(machineId: entry.id, terminalId: $0.terminalId) } }
        guard let index = routes.firstIndex(of: route), routes.indices.contains(index + offset) else { return nil }
        return routes[index + offset]
    }
}

enum Elapsed {
    static func string(sinceMs: UInt64, now: Date) -> String {
        let seconds = max(0, Int(now.timeIntervalSince1970) - Int(sinceMs / 1000))
        switch seconds {
        case ..<60: return "\(seconds)s"
        case ..<3600: return "\(seconds / 60)m"
        case ..<86400: return "\(seconds / 3600)h \(seconds % 3600 / 60)m"
        default: return "\(seconds / 86400)d"
        }
    }

    static func compact(sinceMs: UInt64, now: Date) -> String {
        let minutes = wholeMinutes(sinceMs: sinceMs, now: now)
        switch minutes {
        case ..<1: return "<1m"
        case ..<60: return "\(minutes)m"
        case ..<1440: return "\(minutes / 60)h"
        default: return "\(minutes / 1440)d"
        }
    }

    static func spoken(sinceMs: UInt64, now: Date) -> String {
        let minutes = wholeMinutes(sinceMs: sinceMs, now: now)
        let count = { (n: Int, unit: String) in "\(n) \(unit)\(n == 1 ? "" : "s")" }
        switch minutes {
        case ..<1: return "less than a minute"
        case ..<60: return count(minutes, "minute")
        case ..<1440: return count(minutes / 60, "hour")
        default: return count(minutes / 1440, "day")
        }
    }

    static func spoken(_ status: AgentState, sinceMs: UInt64, now: Date) -> String {
        let amount = spoken(sinceMs: sinceMs, now: now)
        return status == .done ? "done \(amount) ago" : "\(status.label) for \(amount)"
    }

    private static func wholeMinutes(sinceMs: UInt64, now: Date) -> Int {
        max(0, Int(now.timeIntervalSince1970) - Int(sinceMs / 1000)) / 60
    }
}

struct UsageLimit: Equatable {
    let label: String
    let left: UInt8
    let seconds: Int

    var resetsIn: String { PlanUsage.countdown(seconds: seconds) }
}

extension PlanUsage {
    /// The status line runs only while Claude Code is in use, so a quiet Mac's figures age.
    static let staleAfterMs: UInt64 = 5 * 60_000

    func isStale(now: Date) -> Bool {
        let nowMs = UInt64(max(0, now.timeIntervalSince1970 * 1000))
        return nowMs > recordedMs && nowMs - recordedMs > Self.staleAfterMs
    }

    /// A window whose reset time has passed is left out: its figure no longer holds.
    func limits(now: Date) -> [UsageLimit] {
        let nowMs = UInt64(max(0, now.timeIntervalSince1970 * 1000))
        return [("5h", fiveHour), ("7d", sevenDay)].compactMap { label, window in
            guard let window, window.resetsAtMs > nowMs else { return nil }
            return UsageLimit(
                label: label, left: 100 - min(window.usedPercent, 100),
                seconds: Int((window.resetsAtMs - nowMs) / 1000)
            )
        }
    }

    static func countdown(seconds: Int) -> String {
        switch seconds {
        case ..<60: "<1m"
        case ..<3600: "\(seconds / 60)m"
        case ..<86400: "\(seconds / 3600)h \(seconds % 3600 / 60)m"
        default: "\(seconds / 86400)d \(seconds % 86400 / 3600)h"
        }
    }

    /// VoiceOver reads "13m" as metres; this spells the units out.
    static func spoken(seconds: Int) -> String {
        let formatter = DateComponentsFormatter()
        formatter.unitsStyle = .full
        formatter.allowedUnits = seconds < 86400 ? [.hour, .minute] : [.day, .hour]
        formatter.maximumUnitCount = 2
        return formatter.string(from: TimeInterval(max(60, seconds))) ?? ""
    }
}

extension TerminalSummary {
    /// The name given in herdr, else the folder: never a title a program set.
    var displayTitle: String {
        if let label = label?.trimmingCharacters(in: .whitespacesAndNewlines), !label.isEmpty { return label }
        if let cwd, let folder = cwd.split(separator: "/").last { return String(folder) }
        return "Terminal"
    }
}

extension AgentSummary {
    var displayTitle: String {
        [title, name, kind].compactMap { $0?.trimmingCharacters(in: .whitespacesAndNewlines) }.first { !$0.isEmpty }
            ?? terminalId
    }
}
