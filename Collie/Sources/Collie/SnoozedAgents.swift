import CollieCore
import Foundation

struct SnoozedAgents: StateFile {
    struct Snooze: Codable, Equatable {
        let wake: Date
        let statusSinceMs: UInt64

        func ended(by agent: AgentSummary) -> Bool {
            statusSinceMs != agent.statusSinceMs && (agent.status == .blocked || agent.status == .done)
        }
    }

    static let file: URL? = try? StateDirectory.prepare().appending(path: "snoozed-agents.json")

    var snoozes: [AgentRoute: Snooze] = [:]

    func wake(of agent: AgentSummary, route: AgentRoute, now: Date) -> Date? {
        guard let snooze = snoozes[route], snooze.wake > now, !snooze.ended(by: agent) else { return nil }
        return snooze.wake
    }

    mutating func endWoken(_ entries: [MachineFlockEntry]) {
        for entry in entries {
            for agent in entry.agents {
                let route = AgentRoute(machineId: entry.id, terminalId: agent.terminalId)
                if snoozes[route]?.ended(by: agent) == true { snoozes.removeValue(forKey: route) }
            }
        }
    }

    mutating func snooze(_ agent: AgentSummary, route: AgentRoute, until wake: Date) {
        snoozes[route] = Snooze(wake: wake, statusSinceMs: agent.statusSinceMs)
    }

    @discardableResult
    mutating func wakeNow(_ route: AgentRoute) -> Bool {
        snoozes.removeValue(forKey: route) != nil
    }

    mutating func prune(_ entries: [MachineFlockEntry], now: Date) {
        snoozes = snoozes.filter { route, snooze in
            guard snooze.wake > now, let entry = entries.first(where: { $0.id == route.machineId }) else { return false }
            guard entry.error == nil, let flock = entry.flock, flock.link == .connected, flock.details != nil else { return true }
            return flock.agents.first { $0.terminalId == route.terminalId }.map { wake(of: $0, route: route, now: now) != nil } ?? false
        }
    }

    static func forget(machineId: String, file: URL?) {
        var saved = load(from: file)
        saved.snoozes = saved.snoozes.filter { $0.key.machineId != machineId }
        saved.save(to: file)
    }

    static func tomorrowMorning(after now: Date, calendar: Calendar = .current) -> Date {
        let tomorrow = calendar.date(byAdding: .day, value: 1, to: calendar.startOfDay(for: now)) ?? now.addingTimeInterval(24 * 3600)
        return calendar.date(bySettingHour: 9, minute: 0, second: 0, of: tomorrow) ?? tomorrow
    }
}
