import ActivityKit
import CryptoKit
import Foundation
import SwiftUI

// ActivityKit storage always uses ciphertext, including initial attributes and local updates.

struct AgentActivityAttributes: ActivityAttributes {
    static let maxAttributesBytes = 1536
    static let maxStateBytes = 2400
    static let maxTotalBytes = 4000
    let machineId: String
    let terminalId: String
    let nodeId: String
    let bindingId: String
    private let enc: String

    private struct Display: Codable {
        let v: Int
        let machineLabel: String
        let title: String?
    }

    init(machineId: String, terminalId: String, machineLabel: String, nodeId: String, title: String?) {
        self.machineId = machineId
        self.terminalId = terminalId
        self.nodeId = nodeId
        let binding = UUID().uuidString
        bindingId = binding
        let display = Display(v: 1, machineLabel: String(machineLabel.prefix(64)), title: title.map { String($0.prefix(64)) })
        enc = NotificationKey.load(nodeId: nodeId).flatMap {
            PushEnvelope.seal(display, kind: 3, nodeId: nodeId, scope: binding, key: $0)
        } ?? ""
    }

    func fits(_ state: ContentState) -> Bool {
        guard let attributesBytes = try? JSONEncoder().encode(self).count,
            let stateBytes = try? JSONEncoder().encode(state).count
        else { return false }
        return attributesBytes <= Self.maxAttributesBytes && stateBytes <= Self.maxStateBytes
            && attributesBytes + stateBytes <= Self.maxTotalBytes
    }

    private var display: Display? {
        guard let key = NotificationKey.load(nodeId: nodeId),
            let display = PushEnvelope.open(enc, kind: 3, nodeId: nodeId, scope: bindingId, key: key, as: Display.self), display.v == 1
        else { return nil }
        return display
    }

    var machineLabel: String { display?.machineLabel ?? "" }
    var title: String? { display?.title }
    func displayTitle(_ state: ContentState) -> String { state.status == .unknown ? "Collie" : title ?? state.title }

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
        private var sealedDisplay: String? = nil

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

        private struct Display: Codable {
            let v: Int
            let state: PrivateState
        }

        private struct PrivateState: Codable {
            var status: AgentActivityStatus
            var statusSince: Double
            var title: String
            var kind: String?
            var workspace: String?
            var approvals: Int
            var approvalId: String?
            var enc: String?
            var progress: String?
        }

        private enum CodingKeys: String, CodingKey { case v, enc }

        init(from decoder: any Decoder) throws {
            let c = try decoder.container(keyedBy: CodingKeys.self)
            guard try c.decode(Int.self, forKey: .v) == 1 else {
                throw DecodingError.dataCorruptedError(forKey: .v, in: c, debugDescription: "Unsupported encrypted display")
            }
            self.init(status: .unknown, statusSince: Date(timeIntervalSinceReferenceDate: 0), title: "Collie", workspace: nil, approvals: 0)
            sealedDisplay = try c.decode(String.self, forKey: .enc)
        }

        func encode(to encoder: any Encoder) throws {
            var c = encoder.container(keyedBy: CodingKeys.self)
            try c.encode(1, forKey: .v)
            try c.encode(sealedDisplay ?? "", forKey: .enc)
        }

        func secured(nodeId: String, scope: String) -> Self {
            var state = self
            let privateState = PrivateState(status: status, statusSince: statusSince.timeIntervalSinceReferenceDate.rounded(.down),
                title: title, kind: kind, workspace: workspace, approvals: approvals, approvalId: approvalId, enc: enc, progress: progress)
            state.sealedDisplay = NotificationKey.load(nodeId: nodeId).flatMap {
                PushEnvelope.seal(Display(v: 1, state: privateState), kind: 2, nodeId: nodeId, scope: scope, key: $0)
            } ?? ""
            guard let bytes = try? JSONEncoder().encode(state), bytes.count <= AgentActivityAttributes.maxStateBytes else {
                return Self(status: .unknown, statusSince: Date(timeIntervalSinceReferenceDate: 0), title: "Collie", workspace: nil, approvals: 0)
            }
            return state
        }

        func opened(attributes: AgentActivityAttributes) -> Self {
            guard sealedDisplay != nil, let key = NotificationKey.load(nodeId: attributes.nodeId) else {
                return Self(status: .unknown, statusSince: Date(timeIntervalSinceReferenceDate: 0), title: "Collie", workspace: nil, approvals: 0)
            }
            return opened(nodeId: attributes.nodeId, scope: attributes.bindingId, key: key)
        }

        func opened(nodeId: String, scope: String, key: SymmetricKey) -> Self {
            guard let sealedDisplay,
                let display = PushEnvelope.open(sealedDisplay, kind: 2, nodeId: nodeId, scope: scope, key: key, as: Display.self), display.v == 1
            else { return Self(status: .unknown, statusSince: Date(timeIntervalSinceReferenceDate: 0), title: "Collie", workspace: nil, approvals: 0) }
            let d = display.state
            var state = Self(status: d.status, statusSince: Date(timeIntervalSinceReferenceDate: d.statusSince), title: d.title,
                kind: d.kind, workspace: d.workspace, approvals: d.approvals, approvalId: d.approvalId, enc: d.enc)
            state.progress = d.progress
            state.sealedDisplay = sealedDisplay
            return state
        }

        func sameDisplay(as other: Self) -> Bool {
            status == other.status && statusSince == other.statusSince && title == other.title
                && kind == other.kind && workspace == other.workspace && approvals == other.approvals
                && approvalId == other.approvalId && enc == other.enc && progress == other.progress
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
