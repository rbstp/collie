import CollieCore
import Foundation
import Observation

@MainActor
@Observable
final class AgentModel {
    let route: AgentRoute
    private let core: any AgentCore

    private(set) var agent: AgentSummary?
    private(set) var link: LinkPhase?
    private(set) var linkError: String?
    private(set) var ansi = ""
    private var revision: UInt64 = 0

    var draft = ""
    private(set) var sendingPrompt = false
    private(set) var promptError: String?

    private(set) var notice: String?
    private(set) var keyTaps = 0
    private var queuedKeys: [AgentKey] = []
    private var sendingKeys = false

    var close = CloseConfirmation()
    private(set) var closed = false

    // One chain for every screen: a late unwatch from a popped screen must not land after
    // the next screen's watch.
    private static var watchChain: Task<Void, Never>?

    init(core: any AgentCore, route: AgentRoute) {
        self.core = core
        self.route = route
    }

    var canSendPrompt: Bool { !sendingPrompt && !draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty }

    /// Runs while the screen is visible: watch, poll the core at 10 Hz, unwatch on cancel.
    func run() async {
        watch(route.terminalId)
        while !Task.isCancelled {
            poll()
            try? await Task.sleep(for: .milliseconds(100))
        }
        watch(nil)
    }

    func poll() {
        guard let view = core.agentView(machineId: route.machineId, terminalId: route.terminalId, afterRevision: revision) else { return }
        if link != view.link { link = view.link }
        if linkError != view.lastError { linkError = view.lastError }
        if agent != view.agent { agent = view.agent }
        if let output = view.output {
            ansi = output.ansi
            revision = view.outputRevision
        }
    }

    func refresh() async {
        do {
            ansi = try await core.agentRead(machineId: route.machineId, terminalId: route.terminalId, source: .recent).ansi
            notice = nil
        } catch {
            notice = Self.message(for: error)
        }
    }

    func sendPrompt() async {
        let text = draft.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !sendingPrompt, !text.isEmpty else { return }
        sendingPrompt = true
        promptError = nil
        defer { sendingPrompt = false }
        do {
            try await core.prompt(machineId: route.machineId, terminalId: route.terminalId, text: text)
            draft = ""
        } catch {
            promptError = Self.message(for: error)
        }
    }

    /// Keys go out in tap order: taps made while a send is in flight are batched into the next call.
    @discardableResult
    func tap(_ key: AgentKey) -> Task<Void, Never>? {
        keyTaps += 1
        queuedKeys.append(key)
        guard !sendingKeys else { return nil }
        sendingKeys = true
        return Task { await drainKeys() }
    }

    private func drainKeys() async {
        defer { sendingKeys = false }
        while !queuedKeys.isEmpty {
            let batch = Array(queuedKeys.prefix(16))
            queuedKeys.removeFirst(batch.count)
            do {
                try await core.sendKeys(machineId: route.machineId, terminalId: route.terminalId, keys: batch)
                notice = nil
            } catch {
                queuedKeys.removeAll()
                notice = Self.message(for: error)
            }
        }
    }

    func focus() async {
        do {
            try await core.focus(machineId: route.machineId, terminalId: route.terminalId)
            notice = nil
        } catch {
            notice = Self.message(for: error)
        }
    }

    /// Only reachable after both confirmation steps; the core is called with `confirm: true`.
    func performClose() async {
        guard let target = close.confirm() else { return }
        do {
            switch target {
            case .pane:
                try await core.closePane(machineId: route.machineId, terminalId: route.terminalId, confirm: true)
            case .workspace(let workspaceId):
                try await core.closeWorkspace(machineId: route.machineId, workspaceId: workspaceId, confirm: true)
            }
            closed = true
        } catch {
            notice = Self.message(for: error)
        }
    }

    private func watch(_ terminalId: String?) {
        let previous = Self.watchChain
        let core = core
        let machineId = route.machineId
        Self.watchChain = Task {
            await previous?.value
            try? await core.watchAgent(machineId: machineId, terminalId: terminalId)
        }
    }

    static func message(for error: any Error) -> String {
        if let error = error as? CoreError, error == .AgentBlocked {
            return "The agent is waiting for an approval. Approvals from the phone come in Phase 3; answer it on the Mac for now."
        }
        return describe(error)
    }
}

enum CloseTarget: Equatable {
    case pane
    case workspace(id: String)
}

/// Destructive closes need two explicit confirmations before the core sees `confirm: true`.
struct CloseConfirmation: Equatable {
    enum Step: Equatable {
        case idle
        case first(CloseTarget)
        case second(CloseTarget)
    }

    private(set) var step = Step.idle

    var target: CloseTarget? {
        switch step {
        case .idle: nil
        case .first(let target), .second(let target): target
        }
    }

    mutating func begin(_ target: CloseTarget) {
        step = .first(target)
    }

    mutating func advance() {
        if case .first(let target) = step { step = .second(target) }
    }

    mutating func cancel() {
        step = .idle
    }

    mutating func confirm() -> CloseTarget? {
        guard case .second(let target) = step else { return nil }
        step = .idle
        return target
    }
}

extension AgentKey {
    static let strip: [AgentKey] = [.esc, .enter, .up, .down, .tab, .shiftTab, .ctrlC, .y, .n]

    var symbol: String {
        switch self {
        case .esc: "esc"
        case .enter: "⏎"
        case .up: "↑"
        case .down: "↓"
        case .tab: "⇥"
        case .shiftTab: "⇧⇥"
        case .ctrlC: "^C"
        case .y: "y"
        case .n: "n"
        }
    }

    var accessibilityName: String {
        switch self {
        case .esc: "Escape"
        case .enter: "Return"
        case .up: "Up arrow"
        case .down: "Down arrow"
        case .tab: "Tab"
        case .shiftTab: "Shift Tab"
        case .ctrlC: "Control C"
        case .y: "Y"
        case .n: "N"
        }
    }
}
