import ActivityKit
import CryptoKit
import Foundation
import SwiftUI

// Compiled into the app and CollieWidgets. collied sends ContentState as the `content-state` of
// Live Activity pushes: docs/protocol/live-activity-content-state.json is checked by both sides.

struct AgentActivityAttributes: ActivityAttributes {
    let machineId: String
    let terminalId: String
    let machineLabel: String
    /// The Mac's node id, which keys its notification key and its decisions.
    let nodeId: String
    /// The title the app shows for the agent, which may be the terminal title: set on the phone
    /// and never sent through APNs. Nil on activities an older build started.
    let title: String?

    func displayTitle(_ state: ContentState) -> String { title ?? state.title }

    /// Plaintext to Apple: only what the approval alert already shows, plus the status and a count.
    /// `enc` is the approval context sealed like the alert's, so Apple only sees ciphertext.
    struct ContentState: Codable, Hashable, Sendable {
        var status: AgentActivityStatus
        var statusSince: Date
        var title: String
        var kind: String?
        var workspace: String?
        var approvals: Int
        var approvalId: String?
        var enc: String?
        /// Local only, never sent by collied: "Approving…" after a tap, until its next push.
        var progress: String?

        init(
            status: AgentActivityStatus, statusSince: Date, title: String, kind: String? = nil, workspace: String?,
            approvals: Int, approvalId: String? = nil, enc: String? = nil
        ) {
            self.status = status
            self.statusSince = statusSince
            self.title = title
            self.kind = kind
            self.workspace = workspace
            self.approvals = approvals
            self.approvalId = approvalId
            self.enc = enc
        }

        enum CodingKeys: String, CodingKey {
            case status, statusSince, title, kind, workspace, approvals, approvalId, enc, progress
        }

        // statusSince is whole seconds since 2001-01-01 UTC (Foundation's reference date, what a
        // default Date decoding expects), decoded here explicitly so no decoder strategy can change it.
        init(from decoder: any Decoder) throws {
            let container = try decoder.container(keyedBy: CodingKeys.self)
            status = try container.decode(AgentActivityStatus.self, forKey: .status)
            statusSince = Date(timeIntervalSinceReferenceDate: try container.decode(Double.self, forKey: .statusSince))
            title = try container.decode(String.self, forKey: .title)
            kind = try container.decodeIfPresent(String.self, forKey: .kind)
            workspace = try container.decodeIfPresent(String.self, forKey: .workspace)
            approvals = try container.decode(Int.self, forKey: .approvals)
            approvalId = try container.decodeIfPresent(String.self, forKey: .approvalId)
            enc = try container.decodeIfPresent(String.self, forKey: .enc)
            progress = try container.decodeIfPresent(String.self, forKey: .progress)
        }

        func encode(to encoder: any Encoder) throws {
            var container = encoder.container(keyedBy: CodingKeys.self)
            try container.encode(status, forKey: .status)
            try container.encode(Int64(statusSince.timeIntervalSinceReferenceDate.rounded(.down)), forKey: .statusSince)
            try container.encode(title, forKey: .title)
            try container.encodeIfPresent(kind, forKey: .kind)
            try container.encodeIfPresent(workspace, forKey: .workspace)
            try container.encode(approvals, forKey: .approvals)
            try container.encodeIfPresent(approvalId, forKey: .approvalId)
            try container.encodeIfPresent(enc, forKey: .enc)
            try container.encodeIfPresent(progress, forKey: .progress)
        }

        /// The kinds collied sends, each with an icon in CollieWidgets.
        static let kinds = ["claude", "codex", "copilot"]

        /// Matches collied's `relevance-score`: of several followed agents, iOS puts the one that
        /// most needs the user in the Dynamic Island.
        var relevance: Double {
            switch status {
            case .blocked: 100
            case .done: 75
            case .working: 50
            case .idle: 25
            case .unknown: 0
            }
        }

        /// The approval this blocked state can be decided from the activity, if collied sent one.
        var pendingApproval: String? { status == .blocked ? approvalId : nil }

        /// The decrypted command of the pending approval, nil when anything is missing or wrong.
        func command(key: SymmetricKey?) -> String? {
            guard let approvalId = pendingApproval, let enc, let key else { return nil }
            return PushContext.open(enc, approvalId: approvalId, key: key)
        }
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
        guard ProtocolId.valid(machineId, extra: "_-"), ProtocolId.valid(terminalId, extra: "_:.-") else { return nil }
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
}
