import Foundation

/// Prompts not yet sent, per pane, kept on this device only in the state dir rather than UserDefaults.
struct AgentDrafts: Codable {
    struct Draft: Codable {
        var text: String
        var attachments: [AttachedFile]
    }

    static let file: URL? = try? StateDirectory.prepare().appending(path: "drafts.json")

    var drafts: [AgentRoute: Draft] = [:]

    static func load(from file: URL?) -> AgentDrafts {
        file.flatMap { try? Data(contentsOf: $0) }.flatMap { try? JSONDecoder().decode(Self.self, from: $0) } ?? AgentDrafts()
    }

    func save(to file: URL?) {
        guard let file, let data = try? JSONEncoder().encode(self) else { return }
        try? data.write(to: file, options: .atomic)
    }

    /// A draft goes with its pane: once its machine's loaded flock lists it no longer, or the machine is no longer paired.
    static func prune(_ entries: [MachineFlockEntry], file: URL?) {
        var saved = load(from: file)
        let kept = saved.drafts.filter { route, _ in
            guard let entry = entries.first(where: { $0.id == route.machineId }) else { return entries.isEmpty }
            guard let flock = entry.flock, flock.link == .connected, flock.details != nil else { return true }
            return flock.agents.contains { $0.terminalId == route.terminalId } || flock.terminals.contains { $0.terminalId == route.terminalId }
        }
        guard kept.count != saved.drafts.count else { return }
        saved.drafts = kept
        saved.save(to: file)
    }
}
