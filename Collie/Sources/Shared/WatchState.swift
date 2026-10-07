import Foundation
import SwiftUI

// Compiled into the app and both watch targets. The watch gets only what the phone shows:
// never a key, a nonce or a machine address.

struct WatchState: Codable, Equatable, Sendable {
    static let maxApprovals = 5

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

    /// The watch's own check before it asks for the wrist; the phone checks every decision again.
    func canDecide(_ decision: WatchDecision, allowed: Bool, answered: Set<String>, now: Date) -> Bool {
        allowed && !answered.contains(id) && offers(decision) && expiresAtMs > now.unixMs
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
    static let widgetKind = "CollieUsage"

    let fiveHourUsed: UInt8?
    let fiveHourResetsAtMs: UInt64?

    /// Nil once the window's reset time has passed: its figure no longer holds.
    func fiveHour(now: Date) -> UInt8? {
        guard let fiveHourUsed, let reset = fiveHourResetsAtMs, reset > now.unixMs else { return nil }
        return fiveHourUsed
    }

    static let fiveHourLength: TimeInterval = 5 * 3600

    /// Nil, like the figure, once the reset time has passed.
    func fiveHourSecondsLeft(now: Date) -> TimeInterval? {
        guard fiveHour(now: now) != nil, let reset = fiveHourResetsAtMs else { return nil }
        return Date(timeIntervalSince1970: TimeInterval(reset) / 1000).timeIntervalSince(now)
    }

    /// The complication's ring runs down by itself: an entry now, at each ring color change and at the reset.
    func timelineDates(now: Date) -> [Date] {
        guard let left = fiveHourSecondsLeft(now: now) else { return [now] }
        let reset = now.addingTimeInterval(left)
        return [now] + [reset.addingTimeInterval(-7200), reset.addingTimeInterval(-3600), reset].filter { $0 > now }
    }

    static func ringColor(secondsLeft: TimeInterval) -> Color {
        secondsLeft > 7200 ? .green : secondsLeft > 3600 ? .yellow : .red
    }

    static func usedColor(_ used: UInt8) -> Color {
        used <= 60 ? .green : used <= 85 ? .yellow : .red
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
    /// From the watch: answer with the current state, listed from each Mac when the app is not in the foreground.
    static let refresh = "refresh"
    static let title = "title"
    static let body = "body"
    /// In a reply: the watch need not offer this approval again.
    static let answered = "answered"
    /// In a refresh reply: the node ids of the Macs that did not answer.
    static let silent = "silent"
}
