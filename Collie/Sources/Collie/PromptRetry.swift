import CollieCore

/// Only an unreachable machine is retried, and every retry is a core resend, which sends only
/// under the op_id an unanswered identical prompt still holds: collied then replays a prompt that
/// arrived instead of typing it twice. Any answer from collied ends it.
struct PromptRetry {
    static let attempts = 5
    static let interval: Duration = .seconds(60)
    /// A second limit under the core's 300 s op_id window.
    static let window: Duration = .seconds(270)

    let started: ContinuousClock.Instant
    private(set) var attempt = 1
    private var last: ContinuousClock.Instant

    init(started: ContinuousClock.Instant) {
        self.started = started
        last = started
    }

    static func retries(_ error: any Error) -> Bool {
        switch error as? CoreError {
        case .Unreachable?, .NotRunning?: true
        default: false
        }
    }

    func next(after error: any Error) -> ContinuousClock.Instant? {
        guard Self.retries(error), attempt < Self.attempts else { return nil }
        return last + Self.interval
    }

    mutating func begin(at now: ContinuousClock.Instant) -> Bool {
        guard attempt < Self.attempts, now - started <= Self.window else { return false }
        attempt += 1
        last = now
        return true
    }

    /// collied's last prompt is the first non-empty line, cut to 160 characters with "…". It must
    /// have changed since the send started, so a prompt sent twice on purpose is not taken as arrived.
    static func arrived(_ text: String, lastPrompt: String?, before: String?) -> Bool {
        guard let lastPrompt, lastPrompt != before else { return false }
        let line = text.split(whereSeparator: \.isNewline).lazy
            .map { $0.trimmingCharacters(in: .whitespaces) }
            .first { !$0.isEmpty } ?? ""
        return lastPrompt.hasSuffix("…") ? line.hasPrefix(lastPrompt.dropLast()) : line == lastPrompt
    }
}
