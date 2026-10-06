import Foundation

// Compiled into the app and both watch targets. The watch gets only what the phone shows:
// never a key, a nonce or a machine address.

struct WatchState: Codable, Equatable, Sendable {
    var approvals: [WatchApproval]
    var agents: [WatchAgent]
    var usage: WatchUsage?
    var decisionsAllowed: Bool
    var live: Bool
}

struct WatchApproval: Codable, Equatable, Identifiable, Sendable {
    let nodeId: String
    let approvalId: String
    let agent: String
    let place: String
    let command: String?
    let snippet: String
    let options: [WatchDecision]
    let choices: [WatchChoice]
    let answeredInTerminal: Bool
    let expiresAtMs: UInt64

    var id: String { approvalId }

    /// A menu option is only ever chosen where collied offers no Approve/Deny, as on the phone.
    func offers(_ decision: WatchDecision) -> Bool {
        if case .choose(let index) = decision {
            return options.isEmpty && choices.contains { $0.index == index }
        }
        return options.contains(decision)
    }
}

struct WatchChoice: Codable, Equatable, Sendable {
    let index: UInt8
    let label: String
}

/// No Approve always, as on the lock screen.
enum WatchDecision: Codable, Hashable, Sendable {
    case approve
    case deny
    case choose(UInt8)
}

struct WatchAgent: Codable, Equatable, Identifiable, Sendable {
    enum Status: String, Codable, Sendable {
        case idle, working, blocked, done, unknown
    }

    let id: String
    let title: String
    let workspace: String?
    let machine: String
    let status: Status
    let done: Bool
    let activityMs: UInt64
    let contextLeft: UInt8?
}

struct WatchUsage: Codable, Equatable, Sendable {
    let fiveHourUsed: UInt8?
    let fiveHourResetsAtMs: UInt64?

    /// Nil once the window's reset time has passed: its figure no longer holds.
    func fiveHour(now: Date) -> UInt8? {
        guard let fiveHourUsed, let reset = fiveHourResetsAtMs, reset > UInt64(max(0, now.timeIntervalSince1970 * 1000)) else { return nil }
        return fiveHourUsed
    }

    static var file: URL? { AppGroup.container?.appending(path: "watch-usage.json") }

    static func load() -> WatchUsage? {
        file.flatMap { try? Data(contentsOf: $0) }.flatMap { try? JSONDecoder().decode(Self.self, from: $0) }
    }

    func save() {
        guard let file = Self.file, let data = try? JSONEncoder().encode(self) else { return }
        try? data.write(to: file, options: .atomic)
    }

    static func clear() {
        guard let file else { return }
        try? FileManager.default.removeItem(at: file)
    }
}

struct WatchDecisionRequest: Codable, Sendable {
    let nodeId: String
    let approvalId: String
    let decision: WatchDecision
}

enum WatchMessage {
    static let state = "state"
    static let decide = "decide"
    static let title = "title"
    static let body = "body"
    /// In a reply: the watch need not offer this approval again.
    static let answered = "answered"
}
