import ActivityKit
import Foundation
import SwiftUI

// Compiled into the app and CollieWidgets. collied sends ContentState as the `content-state` of
// Live Activity pushes: docs/protocol/live-activity-content-state.json is checked by both sides.

struct AgentActivityAttributes: ActivityAttributes {
    let machineId: String
    let terminalId: String
    let machineLabel: String

    /// Plaintext to Apple: only what the approval alert already shows, plus the status and a count.
    struct ContentState: Codable, Hashable, Sendable {
        var status: AgentActivityStatus
        var statusSince: Date
        var title: String
        var workspace: String?
        var approvals: Int

        init(status: AgentActivityStatus, statusSince: Date, title: String, workspace: String?, approvals: Int) {
            self.status = status
            self.statusSince = statusSince
            self.title = title
            self.workspace = workspace
            self.approvals = approvals
        }

        enum CodingKeys: String, CodingKey {
            case status, statusSince, title, workspace, approvals
        }

        // statusSince is whole seconds since 2001-01-01 UTC (Foundation's reference date, what a
        // default Date decoding expects), decoded here explicitly so no decoder strategy can change it.
        init(from decoder: any Decoder) throws {
            let container = try decoder.container(keyedBy: CodingKeys.self)
            status = try container.decode(AgentActivityStatus.self, forKey: .status)
            statusSince = Date(timeIntervalSinceReferenceDate: try container.decode(Double.self, forKey: .statusSince))
            title = try container.decode(String.self, forKey: .title)
            workspace = try container.decodeIfPresent(String.self, forKey: .workspace)
            approvals = try container.decode(Int.self, forKey: .approvals)
        }

        func encode(to encoder: any Encoder) throws {
            var container = encoder.container(keyedBy: CodingKeys.self)
            try container.encode(status, forKey: .status)
            try container.encode(Int64(statusSince.timeIntervalSinceReferenceDate.rounded(.down)), forKey: .statusSince)
            try container.encode(title, forKey: .title)
            try container.encodeIfPresent(workspace, forKey: .workspace)
            try container.encode(approvals, forKey: .approvals)
        }

        /// Matches collied's `relevance-score`.
        var relevance: Double { status == .blocked ? 100 : 50 }
    }
}

enum AgentActivityStatus: String, Codable, Hashable, Sendable {
    case idle, working, blocked, done, unknown

    /// A status a newer collied adds shows as unknown instead of failing the whole update.
    init(from decoder: any Decoder) throws {
        self = Self(rawValue: try decoder.singleValueContainer().decode(String.self)) ?? .unknown
    }

    var label: String { rawValue }

    var color: Color {
        switch self {
        case .blocked: .red
        case .working: .blue
        case .idle: .gray
        case .done: .green
        case .unknown: .secondary
        }
    }

    var symbol: String {
        switch self {
        case .blocked: "exclamationmark.circle.fill"
        case .working: "circle.dotted.circle"
        case .idle: "pause.circle.fill"
        case .done: "checkmark.circle.fill"
        case .unknown: "questionmark.circle.fill"
        }
    }
}

/// `collie://agent?m=<machine id>&t=<terminal id>`, the Live Activity's tap target.
struct AgentLink: Hashable, Sendable {
    let machineId: String
    let terminalId: String

    init?(machineId: String, terminalId: String) {
        guard Self.valid(machineId, extra: "_-"), Self.valid(terminalId, extra: "_:.-") else { return nil }
        self.machineId = machineId
        self.terminalId = terminalId
    }

    init?(url: URL) {
        guard url.scheme == "collie", url.host() == "agent", url.path().isEmpty || url.path() == "/",
            let items = URLComponents(url: url, resolvingAgainstBaseURL: false)?.queryItems,
            items.count == 2,
            let m = items.first(where: { $0.name == "m" })?.value,
            let t = items.first(where: { $0.name == "t" })?.value
        else { return nil }
        self.init(machineId: m, terminalId: t)
    }

    var url: URL {
        var components = URLComponents()
        components.scheme = "collie"
        components.host = "agent"
        components.queryItems = [URLQueryItem(name: "m", value: machineId), URLQueryItem(name: "t", value: terminalId)]
        return components.url!
    }

    /// The protocol's id grammar: ASCII letters, digits and `extra`, 1 to 64 bytes.
    private static func valid(_ s: String, extra: String) -> Bool {
        (1...64).contains(s.utf8.count)
            && s.unicodeScalars.allSatisfy { c in
                ("a"..."z").contains(c) || ("A"..."Z").contains(c) || ("0"..."9").contains(c) || extra.unicodeScalars.contains(c)
            }
    }
}
