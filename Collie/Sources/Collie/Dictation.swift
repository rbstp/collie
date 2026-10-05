import Foundation
import Observation
import SwiftUI

enum DictationLanguage: String, Codable, CaseIterable {
    case english = "en-US"
    case french = "fr-CA"

    var locale: Locale { Locale(identifier: rawValue) }

    var label: String {
        switch self {
        case .english: "English (US)"
        case .french: "French (Canada)"
        }
    }

    var code: String {
        switch self {
        case .english: "EN"
        case .french: "FR"
        }
    }

    var other: DictationLanguage { self == .english ? .french : .english }
}

enum DictationEvent: Equatable, Sendable {
    case level(Float)
    case volatile(String)
    case final(String)
}

enum DictationError: Error {
    case unavailable
}

/// One listening session. Cancelling a task that iterates `events` does not stop the microphone; `cancel` does.
struct DictationSession {
    let events: AsyncThrowingStream<DictationEvent, any Error>
    /// Stops listening; `events` then delivers the last results and ends.
    let finish: @MainActor () async -> Void
    let cancel: @MainActor () -> Void
}

/// Audio must never leave the phone: an engine transcribes on the device only.
@MainActor
protocol DictationEngine: AnyObject {
    /// Asks on first use; false once access is refused.
    func requestMicrophone() async -> Bool
    /// Downloads the language's model when it is missing; throws `DictationError.unavailable` when the device has none.
    func install(_ language: DictationLanguage, progress: @escaping @MainActor (Double) -> Void) async throws
    func start(_ language: DictationLanguage) async throws -> DictationSession
}

enum DictationProblem: Equatable {
    case microphoneDenied
    case unavailable(DictationLanguage)
    case failed(String)

    var message: String {
        switch self {
        case .microphoneDenied: "Microphone access is off. Turn it on in Settings to dictate."
        case .unavailable(let language): "On-device dictation in \(language.label) is not available on this device."
        case .failed(let reason): "Dictation stopped: \(reason)"
        }
    }
}

/// Dictated text goes after what the field held at the start; volatile results are replaced as they firm up.
@MainActor
@Observable
final class DictationModel {
    enum Phase: Equatable {
        case idle
        case preparing(download: Double?)
        case listening
        case finishing
    }

    private(set) var phase = Phase.idle
    private(set) var level: Float = 0
    var problem: DictationProblem?
    private(set) var language: DictationLanguage

    private let engine: any DictationEngine
    private let prefsFile: URL?
    private var generation = 0
    private var task: Task<Void, Never>?
    private var session: DictationSession?
    private var base = ""
    private var finalized = ""
    private var volatile = ""
    private var write: (@MainActor (String) -> Void)?

    init(engine: any DictationEngine, language: DictationLanguage, prefsFile: URL?) {
        self.engine = engine
        self.language = language
        self.prefsFile = prefsFile
    }

    var isActive: Bool { phase != .idle }

    var output: String { Self.join(base, Self.join(finalized, volatile)) }

    @discardableResult
    func start(appendingTo text: String, write: @escaping @MainActor (String) -> Void) -> Task<Void, Never>? {
        guard phase == .idle else { return nil }
        generation += 1
        base = text
        finalized = ""
        volatile = ""
        problem = nil
        self.write = write
        phase = .preparing(download: nil)
        let previous = self.task
        let task = Task { [generation, language] in await run(generation, language, after: previous) }
        self.task = task
        return task
    }

    /// Waits for `previous` to release the shared audio session before opening its own.
    private func run(_ id: Int, _ language: DictationLanguage, after previous: Task<Void, Never>?) async {
        do {
            guard await engine.requestMicrophone() else {
                if id == generation {
                    problem = .microphoneDenied
                    end()
                }
                return
            }
            guard id == generation else { return }
            try await engine.install(language) { [weak self] fraction in
                guard let self, id == self.generation, case .preparing = self.phase else { return }
                self.phase = .preparing(download: fraction)
            }
            await previous?.value
            guard id == generation else { return }
            let session = try await engine.start(language)
            guard id == generation else {
                session.cancel()
                return
            }
            self.session = session
            phase = .listening
            for try await event in session.events {
                guard id == generation else { return }
                apply(event)
            }
        } catch DictationError.unavailable {
            if id == generation { problem = .unavailable(language) }
        } catch {
            if id == generation, !(error is CancellationError) { problem = .failed(error.localizedDescription) }
        }
        if id == generation { end() }
    }

    private func apply(_ event: DictationEvent) {
        switch event {
        case .level(let value):
            if phase == .listening { level = value }
        case .volatile(let text):
            volatile = text
            write?(output)
        case .final(let text):
            finalized = Self.join(finalized, text)
            volatile = ""
            write?(output)
        }
    }

    /// Keeps what was heard; a start still preparing is dropped.
    func stop() {
        switch phase {
        case .idle, .finishing:
            return
        case .preparing:
            cancel()
        case .listening:
            phase = .finishing
            level = 0
            if let session { Task { await session.finish() } }
        }
    }

    func cancel() {
        guard phase != .idle else { return }
        generation += 1
        task?.cancel()
        end()
    }

    /// Remembered on this device; a running dictation carries on in the new language after the text so far.
    func select(_ language: DictationLanguage) {
        guard language != self.language else { return }
        self.language = language
        problem = nil
        var prefs = DevicePrefs.load(from: prefsFile)
        prefs.dictationLanguage = language
        prefs.save(to: prefsFile)
        guard phase != .idle, phase != .finishing, let write else { return }
        let text = output
        cancel()
        start(appendingTo: text, write: write)
    }

    func scenePhaseChanged(to phase: ScenePhase) {
        if phase == .background { stop() }
    }

    private func end() {
        phase = .idle
        level = 0
        session?.cancel()
        session = nil
        write = nil
    }

    static func join(_ head: String, _ tail: String) -> String {
        guard let last = head.last, let first = tail.first, !last.isWhitespace, !first.isWhitespace else { return head + tail }
        return head + " " + tail
    }
}
