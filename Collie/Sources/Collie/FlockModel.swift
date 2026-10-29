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

    /// Waiting for a dial or the first list, or for the node while `nodeStarting`, and no read failed.
    func connecting(nodeStarting: Bool) -> Bool {
        guard error == nil else { return false }
        switch flock?.link {
        case nil, .connecting: return true
        case .offline: return nodeStarting
        case .connected: return flock?.details == nil
        default: return false
        }
    }

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
    private(set) var archiving = false
    private(set) var archiveNotice: String?
    private var archiveNoticeTask: Task<Void, Never>?
    private let archiveNoticeLifetime: Duration

    init(archiveNoticeLifetime: Duration = .seconds(10)) {
        self.archiveNoticeLifetime = archiveNoticeLifetime
    }

    func beginClose(_ target: CloseTarget, route: AgentRoute) {
        guard !archiving else { return }
        closing = route
        close.begin(target)
    }

    /// Only reachable after the confirmation, as on the agent screen.
    func performClose(core: any AgentCore) async -> Bool {
        guard !archiving, let route = closing, let target = close.confirm() else { return false }
        archiving = target == .archive
        defer { archiving = false }
        closeNotice = nil
        clearArchiveNotice()
        do {
            if let message = try await core.closeConfirmed(target, route: route) {
                showArchiveNotice(message)
            }
            return true
        } catch {
            let message = AgentModel.message(for: error)
            let label = entries.first { $0.id == route.machineId }?.machine.label
            closeNotice = label.map { "\($0): \(message)" } ?? message
            return false
        }
    }

    func showArchiveNotice(_ message: String) {
        closeNotice = nil
        clearArchiveNotice()
        archiveNotice = ArchiveSummary.message(message)
        archiveNoticeTask = Task { [weak self, archiveNoticeLifetime] in
            do {
                try await Task.sleep(for: archiveNoticeLifetime)
            } catch {
                return
            }
            self?.archiveNotice = nil
        }
    }

    private func clearArchiveNotice() {
        archiveNoticeTask?.cancel()
        archiveNoticeTask = nil
        archiveNotice = nil
    }

    /// Without `snapshot`, a machine is read from the core's cache, unless its last read failed;
    /// `cacheOnly` then leaves that machine as it is, so nothing goes over the network.
    func refresh(core: (any FlockCore)?, machines: [Machine]? = nil, snapshot: Bool = true, cacheOnly: Bool = false) async {
        guard let core, !refreshing else { return }
        refreshing = true
        defer { refreshing = false }
        let seeded = (machines ?? core.machines()).map { machine in
            entries.first { $0.id == machine.id }.map { MachineFlockEntry(machine: machine, flock: $0.flock, error: $0.error) }
                ?? MachineFlockEntry(machine: machine)
        }
        if seeded != entries { entries = seeded }
        await withTaskGroup(of: MachineFlockEntry.self) { group in
            for entry in seeded where !(cacheOnly && entry.error != nil) {
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

    static func routes(in entries: [MachineFlockEntry]) -> [AgentRoute] {
        entries.flatMap { entry in entry.agents.map { AgentRoute(machineId: entry.id, terminalId: $0.terminalId) } }
    }

    /// Steps through `order`, past agents no longer in `entries`; the current agent keeps its place even when gone.
    static func neighbor(of route: AgentRoute, offset: Int, in order: [AgentRoute], among entries: [MachineFlockEntry]) -> AgentRoute? {
        let live = Set(routes(in: entries))
        let routes = order.filter { $0 == route || live.contains($0) }
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

enum UsagePace: Equatable {
    case slower
    case onPace
    case faster
}

struct UsageLimit: Equatable {
    let label: String
    let used: UInt8
    let seconds: Int
    let length: Int

    var left: UInt8 { 100 - used }

    var title: String { label == "5h" ? "5-hour usage limit" : "Weekly usage limit" }

    /// The share of the window that has passed, from its reset time and its length.
    var elapsed: Double { min(1, max(0, 1 - Double(seconds) / Double(length))) }

    /// Within 5 points of the elapsed share counts as on pace.
    var pace: UsagePace {
        let gap = Double(used) / 100 - elapsed
        return gap < -0.05 ? .slower : gap > 0.05 ? .faster : .onPace
    }

    /// A clock time for the 5-hour window, a countdown for the weekly one.
    func resets(now: Date, style: Date.FormatStyle = .init(date: .omitted, time: .shortened)) -> String {
        label == "5h" ? now.addingTimeInterval(TimeInterval(seconds)).formatted(style) : PlanUsage.countdown(seconds: seconds)
    }

    func resetsLine(now: Date) -> String {
        label == "5h" ? "resets at \(resets(now: now))" : "resets in \(resets(now: now))"
    }

    func spokenResetsLine(now: Date) -> String {
        label == "5h" ? resetsLine(now: now) : "resets in \(PlanUsage.spoken(seconds: seconds))"
    }

    func spokenLine(now: Date) -> String {
        let resets = spokenResetsLine(now: now)
        let trend = switch pace {
        case .slower: "usage pace slower"
        case .onPace: "usage on pace"
        case .faster: "usage pace faster"
        }
        return "\(title), \(used) percent used, \(Int((elapsed * 100).rounded())) percent of the window elapsed, \(trend), \(resets)"
    }
}

extension PlanUsage {
    /// The status line runs only while Claude Code is in use, so a quiet Mac's figures age.
    static let staleAfterMs: UInt64 = 5 * 60_000

    func isStale(now: Date) -> Bool {
        let nowMs = now.unixMs
        return nowMs > recordedMs && nowMs - recordedMs > Self.staleAfterMs
    }

    /// A window whose reset time has passed is left out: its figure no longer holds.
    func limits(now: Date) -> [UsageLimit] {
        let nowMs = now.unixMs
        return [("5h", fiveHour, 5 * 3600), ("7d", sevenDay, 7 * 86400)].compactMap { label, window, length in
            guard let window, window.resetsAtMs > nowMs else { return nil }
            return UsageLimit(
                label: label, used: min(window.usedPercent, 100),
                seconds: Int((window.resetsAtMs - nowMs) / 1000), length: length
            )
        }
    }

    func age(now: Date) -> String {
        Self.age(sinceMs: max(recordedMs, codex?.recordedMs ?? 0), now: now)
    }

    func spokenAge(now: Date) -> String {
        Self.spokenAge(sinceMs: max(recordedMs, codex?.recordedMs ?? 0), now: now)
    }

    static func age(sinceMs: UInt64, now: Date) -> String {
        ageSeconds(sinceMs, now: now) < 60 ? "just now" : "\(Elapsed.string(sinceMs: sinceMs, now: now)) ago"
    }

    static func spokenAge(sinceMs: UInt64, now: Date) -> String {
        let seconds = ageSeconds(sinceMs, now: now)
        return seconds < 60 ? "recorded just now" : "recorded \(spoken(seconds: seconds)) ago"
    }

    /// New task's line for the agent's fullest window at 80% or more. Copilot CLI has no plan data.
    func warning(agent: String, now: Date) -> PlanUsageWarning? {
        let name: String, used: UInt8, resets: String, spokenResets: String, sinceMs: UInt64, stale: Bool
        switch agent {
        case "claude":
            guard let limit = limits(now: now).max(by: { ($0.used, $0.seconds) < ($1.used, $1.seconds) }) else { return nil }
            name = limit.label == "5h" ? "Claude 5-hour limit" : "Claude weekly limit"
            used = limit.used
            resets = limit.resetsLine(now: now)
            spokenResets = limit.spokenResetsLine(now: now)
            sinceMs = recordedMs
            stale = isStale(now: now)
        case "codex":
            guard let codex, codex.resetsAtMs > now.unixMs else { return nil }
            name = "Codex monthly limit"
            used = codex.percent
            resets = codex.resetsLine
            spokenResets = resets
            sinceMs = codex.recordedMs
            stale = codex.isStale(now: now)
        default:
            return nil
        }
        guard used >= 80 else { return nil }
        let head = used >= 100 ? "\(name) reached, " : "\(name) at \(used)%, "
        return PlanUsageWarning(
            text: head + resets + (stale ? ", recorded \(Self.age(sinceMs: sinceMs, now: now))" : ""),
            spoken: head + spokenResets + (stale ? ", \(Self.spokenAge(sinceMs: sinceMs, now: now))" : ""),
            reached: used >= 100
        )
    }

    private static func ageSeconds(_ ms: UInt64, now: Date) -> Int {
        max(0, Int(now.timeIntervalSince1970) - Int(ms / 1000))
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

    var activityMs: UInt64 { lastActivityMs ?? statusSinceMs }
}

struct PlanUsageWarning: Equatable {
    let text: String
    let spoken: String
    let reached: Bool
}

extension CodexUsage {
    static let staleAfterMs: UInt64 = 10 * 60_000

    var percent: UInt8 { limit > 0 ? UInt8(min(100, (Double(used) / Double(limit) * 100).rounded())) : 0 }

    var resetsLine: String { "resets \(Date(unixMs: resetsAtMs).formatted(date: .abbreviated, time: .omitted))" }

    func isStale(now: Date) -> Bool { now.unixMs > recordedMs + Self.staleAfterMs }

    func elapsed(now: Date) -> Double {
        let end = Date(unixMs: resetsAtMs)
        guard let interval = WatchUsage.monthlyInterval(endingAt: end) else { return 0 }
        return min(1, max(0, now.timeIntervalSince(interval.lowerBound) / end.timeIntervalSince(interval.lowerBound)))
    }
}

enum ArchiveSummary {
    static func message(_ result: String) -> String {
        var details: [String] = []
        if result.contains("reported a different path") {
            details.append("The removed worktree did not match the expected path. Branch cleanup was skipped.")
        }
        if result.contains("workspace closure could not be verified") {
            details.append("The workspace may still be open.")
        }
        if result.contains("Authorization ended") {
            details.append("Branch cleanup was skipped because the connection was no longer authorized.")
        } else if result.contains("branch cleanup may be partial") {
            details.append("Some branches could not be cleaned up.")
        }
        let knownResult = result.hasPrefix("Closed pane.") || result.hasPrefix("Removed worktree checkout ")
            || result == "Archive complete"
        if details.isEmpty && !knownResult {
            details.append("Some cleanup steps could not be confirmed.")
        }
        return (["Archive complete"] + details).joined(separator: "\n")
    }
}
