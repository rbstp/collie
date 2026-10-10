import CollieCore
import Foundation

struct SeenAgents: StateFile {
    struct Marker: Codable {
        let statusSinceMs: UInt64
        let wasDone: Bool
        let seenAtMs: UInt64
    }

    static let file: URL? = try? StateDirectory.prepare().appending(path: "seen-agents.json")
    static let limit = 512

    var markers: [AgentRoute: Marker] = [:]

    func isSeen(_ agent: AgentSummary, route: AgentRoute) -> Bool {
        agent.status == .done && markers[route]?.wasDone == true && markers[route]?.statusSinceMs == agent.statusSinceMs
    }

    mutating func markOpened(_ agent: AgentSummary, route: AgentRoute, nowMs: UInt64) {
        markers[route] = Marker(statusSinceMs: agent.statusSinceMs, wasDone: agent.status == .done, seenAtMs: nowMs)
        if markers.count > Self.limit, let oldest = markers.min(by: { $0.value.seenAtMs < $1.value.seenAtMs })?.key {
            markers.removeValue(forKey: oldest)
        }
    }

    mutating func markUnseen(_ agent: AgentSummary, route: AgentRoute) {
        if isSeen(agent, route: route) { markers.removeValue(forKey: route) }
    }

    mutating func prune(_ entries: [MachineFlockEntry]) {
        markers = markers.filter { route, _ in
            guard let entry = entries.first(where: { $0.id == route.machineId }) else { return false }
            guard entry.error == nil, let flock = entry.flock, flock.link == .connected, flock.details != nil else { return true }
            return flock.agents.contains { $0.terminalId == route.terminalId }
                || flock.terminals.contains { $0.terminalId == route.terminalId }
        }
    }

    static func forget(machineId: String, file: URL?) {
        var saved = load(from: file)
        saved.markers = saved.markers.filter { $0.key.machineId != machineId }
        saved.save(to: file)
    }
}
