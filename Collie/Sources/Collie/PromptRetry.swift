import CollieCore
import Foundation
import Observation

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
    /// have changed from a known one since the send started, so a prompt sent twice on purpose, or
    /// while the last one was unknown, is not taken as arrived.
    static func arrived(_ text: String, lastPrompt: String?, before: String?) -> Bool {
        guard let before, let lastPrompt, lastPrompt != before else { return false }
        let line = text.split(whereSeparator: \.isNewline).lazy
            .map { $0.trimmingCharacters(in: .whitespaces) }
            .first { !$0.isEmpty } ?? ""
        return lastPrompt.hasSuffix("…") ? line.hasPrefix(lastPrompt.dropLast()) : line == lastPrompt
    }
}

/// One pending resend per agent, owned by the app so it goes on after the agent screen closes, for as long
/// as the process runs (nothing is kept across launches). A screen that opens shows the pending prompt.
/// With no screen at the end, a prompt that may not have arrived goes back to the agent's draft.
@MainActor
@Observable
final class PromptResends {
    static let shared = PromptResends()

    struct Send {
        let core: any AgentCore
        let route: AgentRoute
        let text: String
        let expected: String?
        let before: String?
        let draftsFile: URL?
        let now: () -> ContinuousClock.Instant
        let sleep: @Sendable (Duration) async throws -> Void
    }

    @Observable
    final class Entry {
        let sent: String
        let files: [AttachedFile]
        fileprivate(set) var stopped = false
        @ObservationIgnored weak var viewer: AgentModel?
        @ObservationIgnored fileprivate var wait: Task<Void, any Error>?
        /// Nil once the prompt arrived.
        @ObservationIgnored fileprivate(set) var outcome: Task<(any Error)?, Never>!

        fileprivate init(sent: String, files: [AttachedFile]) {
            self.sent = sent
            self.files = files
        }
    }

    private(set) var entries: [AgentRoute: Entry] = [:]
    @ObservationIgnored private var unseen: [AgentRoute: String] = [:]

    func start(_ send: Send, sent: String, files: [AttachedFile], retry: PromptRetry, error: any Error, linked: Bool, viewer: AgentModel?) -> Entry {
        let entry = Entry(sent: sent, files: files)
        entry.viewer = viewer
        entries[send.route] = entry
        entry.outcome = Task { await self.run(send, entry, retry: retry, error: error, linked: linked) }
        return entry
    }

    /// An attempt in flight still completes; no other starts.
    func stop(_ route: AgentRoute) {
        guard let entry = entries[route], !entry.stopped else { return }
        entry.stopped = true
        entry.wait?.cancel()
    }

    func stop(machineId: String) {
        for route in entries.keys where route.machineId == machineId {
            stop(route)
            entries[route] = nil
        }
    }

    func wake(_ route: AgentRoute) {
        entries[route]?.wait?.cancel()
    }

    func leave(_ route: AgentRoute, _ viewer: AgentModel) {
        if entries[route]?.viewer === viewer { entries[route]?.viewer = nil }
    }

    func takeUnseen(_ route: AgentRoute) -> String? {
        unseen.removeValue(forKey: route)
    }

    private func run(_ send: Send, _ entry: Entry, retry: PromptRetry, error: any Error, linked: Bool) async -> (any Error)? {
        let outcome = await resend(send, entry, retry: retry, error: error, linked: linked)
        guard entries[send.route] === entry else { return outcome }
        entries[send.route] = nil
        if let outcome, entry.viewer == nil {
            if let file = send.draftsFile {
                var saved = AgentDrafts.load(from: file)
                if saved.drafts[send.route] == nil {
                    saved.drafts[send.route] = AgentDrafts.Draft(text: entry.sent, attachments: entry.files, shell: false)
                    saved.save(to: file)
                }
            }
            unseen[send.route] = AgentModel.promptMessage(for: outcome)
        }
        return outcome
    }

    private func resend(_ send: Send, _ entry: Entry, retry: PromptRetry, error: any Error, linked: Bool) async -> (any Error)? {
        var retry = retry
        var error = error
        var linked = linked
        let view = { send.core.agentView(machineId: send.route.machineId, terminalId: send.route.terminalId, afterRevision: .max) }
        while true {
            guard !entry.stopped, let at = retry.next(after: error) else { return error }
            if linked || view()?.link != .connected {
                let delay = at - send.now()
                let wait = Task { try await send.sleep(delay) }
                entry.wait = wait
                _ = try? await wait.value
                entry.wait = nil
            }
            guard !entry.stopped, retry.begin(at: send.now()) else { return error }
            if PromptRetry.arrived(send.text, lastPrompt: view()?.agent?.lastPrompt, before: send.before) { return nil }
            linked = view()?.link == .connected
            do {
                try await send.core.resendPrompt(
                    machineId: send.route.machineId, terminalId: send.route.terminalId, text: send.text, expectedDraft: send.expected
                )
                return nil
            } catch let failure {
                error = failure
            }
        }
    }
}
