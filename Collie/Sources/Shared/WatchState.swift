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

    var sevenDayUsed: UInt8? = nil
    var sevenDayResetsAtMs: UInt64? = nil
    var codexUsed: UInt8? = nil
    var codexResetsAtMs: UInt64? = nil

    struct Window {
        let used: UInt8
        let interval: ClosedRange<Date>
    }

    func windows(now: Date) -> [Window?] {
        [
            window(used: fiveHourUsed, reset: fiveHourResetsAtMs, length: 5 * 3600, now: now),
            window(used: sevenDayUsed, reset: sevenDayResetsAtMs, length: 7 * 24 * 3600, now: now),
            window(used: codexUsed, reset: codexResetsAtMs, length: nil, now: now),
        ]
    }

    private func window(used: UInt8?, reset: UInt64?, length: TimeInterval?, now: Date) -> Window? {
        guard let used, let reset, reset > now.unixMs else { return nil }
        let end = Date(timeIntervalSince1970: TimeInterval(reset) / 1000)
        let start: Date
        if let length {
            start = end.addingTimeInterval(-length)
        } else {
            var calendar = Calendar(identifier: .gregorian)
            calendar.timeZone = TimeZone(secondsFromGMT: 0)!
            guard let previousMonth = calendar.date(byAdding: .month, value: -1, to: end) else { return nil }
            start = previousMonth
        }
        return Window(used: used, interval: start...end)
    }

    func timelineDates(now: Date) -> [Date] {
        [now] + Set(windows(now: now).compactMap { $0?.interval.upperBound }).sorted()
    }

    static func usedColor(_ used: UInt8) -> Color {
        let stops: [(Double, Color)] = [
            (0, Color(red: 0.15, green: 1, blue: 0.3)),
            (60, Color(red: 1, green: 0.85, blue: 0)),
            (80, Color(red: 1, green: 0.45, blue: 0)),
            (90, Color(red: 1, green: 0.15, blue: 0.1)),
            (100, Color(red: 0.65, green: 0.02, blue: 0.08)),
        ]
        let percent = Double(min(used, 100))
        for (lower, upper) in zip(stops, stops.dropFirst()) where percent <= upper.0 {
            return lower.1.mix(with: upper.1, by: (percent - lower.0) / (upper.0 - lower.0))
        }
        return stops[stops.count - 1].1
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
