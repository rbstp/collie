import CollieCore
import Foundation

struct GridOrder: StateFile, Equatable {
    static let file: URL? = try? StateDirectory.prepare().appending(path: "grid-order.json")

    private(set) var routes: [AgentRoute] = []

    mutating func update(_ entries: [MachineFlockEntry]) {
        routes.removeAll { route in
            guard let entry = entries.first(where: { $0.id == route.machineId }),
                  entry.error == nil, let flock = entry.flock, flock.link == .connected, flock.details != nil
            else { return false }
            return !flock.agents.contains { $0.terminalId == route.terminalId }
        }
        var known = Set(routes)
        for entry in entries {
            for agent in entry.agents {
                let route = AgentRoute(machineId: entry.id, terminalId: agent.terminalId)
                if known.insert(route).inserted { routes.append(route) }
            }
        }
    }

    func agents(in entry: MachineFlockEntry) -> [AgentSummary] {
        let agents = entry.agents
        let byId = Dictionary(uniqueKeysWithValues: agents.map { ($0.terminalId, $0) })
        let saved = routes.filter { $0.machineId == entry.id }
        let known = Set(saved.map(\.terminalId))
        return saved.compactMap { byId[$0.terminalId] } + agents.filter { !known.contains($0.terminalId) }
    }

    mutating func move(_ source: AgentRoute, to destination: AgentRoute?, in entry: MachineFlockEntry, starred: Set<AgentRoute>, after: Bool = false) -> Bool {
        guard source.machineId == entry.id,
              entry.agents.contains(where: { $0.terminalId == source.terminalId })
        else { return false }
        if let destination {
            guard source != destination, destination.machineId == entry.id,
                  starred.contains(source) == starred.contains(destination),
                  entry.agents.contains(where: { $0.terminalId == destination.terminalId })
            else { return false }
        }
        update([entry])
        routes.removeAll { $0 == source }
        let group = routes.filter { $0.machineId == entry.id && starred.contains($0) == starred.contains(source) }
        let anchor = destination ?? (after ? group.last : group.first)
        let index = anchor.flatMap { routes.firstIndex(of: $0) }.map { $0 + (after ? 1 : 0) } ?? routes.endIndex
        routes.insert(source, at: index)
        return true
    }
}
