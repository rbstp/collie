import CollieCore
import Foundation

struct MachineOrder: StateFile, Equatable {
    static let file: URL? = try? StateDirectory.prepare().appending(path: "machine-order.json")

    var ids: [String] = []

    func sorted(_ machines: [Machine]) -> [Machine] {
        let byId = Dictionary(uniqueKeysWithValues: machines.map { ($0.id, $0) })
        let known = Set(ids)
        return ids.compactMap { byId[$0] } + machines.filter { !known.contains($0.id) }
    }
}
