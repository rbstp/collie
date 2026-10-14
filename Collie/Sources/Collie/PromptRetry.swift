import CollieCore

/// Resends a prompt only while the machine is unreachable. The core then keeps the prompt's
/// op_id for an identical request (same text and expected draft) for 300 s, so collied replays
/// an attempt that did arrive instead of typing the prompt twice. Any answer from collied ends it.
struct PromptRetry {
    static let attempts = 5
    static let interval: Duration = .seconds(60)
    /// Under the core's 300 s reuse window. The continuous clock counts while the phone sleeps,
    /// which the core's clock does not.
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

    /// When to send again after `error`, or nil to give up.
    func next(after error: any Error) -> ContinuousClock.Instant? {
        guard Self.retries(error), attempt < Self.attempts else { return nil }
        return last + Self.interval
    }

    /// Counts an attempt starting `now`, which is early when the machine came back.
    mutating func begin(at now: ContinuousClock.Instant) -> Bool {
        guard attempt < Self.attempts, now - started <= Self.window else { return false }
        attempt += 1
        last = now
        return true
    }
}
