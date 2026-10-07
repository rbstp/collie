import Foundation

// Compiled into the app, ColliePush, CollieWidgets and the watch targets: Foundation only, extension-safe APIs only.

extension Date {
    var unixMs: UInt64 { UInt64(max(0, timeIntervalSince1970 * 1000)) }
}

enum AppGroup {
    static let identifier = "group.dev.rbstp.collie"

    static var container: URL? {
        FileManager.default.containerURL(forSecurityApplicationGroupIdentifier: identifier)
    }
}

/// One Mac's entry in `reachability.json`, which collie-core keeps in the App Group container.
struct MacReachability: Decodable, Equatable {
    static let fileName = "reachability.json"

    var lastOkMs: UInt64?
    var lastFailMs: UInt64?

    enum CodingKeys: String, CodingKey {
        case lastOkMs = "last_ok_ms"
        case lastFailMs = "last_fail_ms"
    }

    var lastSeenUnreachable: Bool {
        guard let fail = lastFailMs else { return false }
        return fail > (lastOkMs ?? 0)
    }

    static func parse(_ data: Data) -> [String: MacReachability] {
        (try? JSONDecoder().decode([String: MacReachability].self, from: data)) ?? [:]
    }
}

enum PushBody {
    static let unreachableSuffix = " (machine may be unreachable, open collie to check)"

    static func rewrite(_ body: String, nodeId: String?, reachability: Data?) -> String {
        guard let nodeId, let reachability, !body.hasSuffix(unreachableSuffix),
            MacReachability.parse(reachability)[nodeId]?.lastSeenUnreachable == true
        else { return body }
        return body + unreachableSuffix
    }
}
