import CollieCore
import Foundation

struct GitViewed: StateFile {
    struct Mark: Codable, Hashable {
        let route: AgentRoute
        let root: String
        let path: String
        let hash: String
    }

    static let file: URL? = try? StateDirectory.prepare().appending(path: "git-viewed.json")
    static let limit = 2048

    var marks: [Mark: UInt64] = [:]

    static func mark(route: AgentRoute, root: String, file: GitFile) -> Mark? {
        file.hash.map { Mark(route: route, root: root, path: file.path, hash: $0) }
    }

    mutating func set(_ mark: Mark, viewed: Bool, nowMs: UInt64) {
        marks[mark] = viewed ? nowMs : nil
        if marks.count > Self.limit, let oldest = marks.min(by: { $0.value < $1.value })?.key {
            marks.removeValue(forKey: oldest)
        }
    }

    static func forget(machineId: String, file: URL?) {
        var saved = load(from: file)
        guard saved.marks.keys.contains(where: { $0.route.machineId == machineId }) else { return }
        saved.marks = saved.marks.filter { $0.key.route.machineId != machineId }
        saved.save(to: file)
    }
}
