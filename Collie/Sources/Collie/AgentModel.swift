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
    private(set) var refreshing = false
    private var revision: UInt64 = 0

    var draft = ""
    /// What the phone last saw in the Mac's input box: nil when unknown. A send replaces exactly this text.
    private(set) var macDraft: String?
    private(set) var sendingPrompt = false
    private(set) var promptError: String?
    private(set) var upload: AttachmentUpload?
    private var uploadTask: Task<Void, Never>?
    private(set) var attachments: [AttachedFile] = []

    /// nil while no approval blocks the agent; collied refuses keys and text on the rest.
    var blocked: BlockedInput?
    private(set) var notice: String?
    private(set) var keyTaps = 0
    private var queuedKeys: [AgentKey] = []
    private var sendingKeys = false

    var close = CloseConfirmation()
    private(set) var closed = false

    private let prefsFile: URL?
    var wrapLines: Bool {
        didSet {
            var prefs = DevicePrefs.load(from: prefsFile)
            prefs.wrapLines = wrapLines
            prefs.save(to: prefsFile)
        }
    }

    var keepsKeyboard: Bool { DevicePrefs.load(from: prefsFile).keepKeyboard }

    // One chain for every screen: a late unwatch from a popped screen must not land after
    // the next screen's watch.
    private static var watchChain: Task<Void, Never>?

    init(core: any AgentCore, route: AgentRoute, prefsFile: URL? = DevicePrefs.file) {
        self.core = core
        self.route = route
        self.prefsFile = prefsFile
        wrapLines = DevicePrefs.load(from: prefsFile).wrapLines
    }

    var acceptsKeys: Bool { blocked != .optionsOnly }

    /// Typed text answers the blocking prompt instead of prompting the agent.
    var answering: Bool { blocked == .keysAndText }

    var blockedHint: String? {
        switch blocked {
        case nil: nil
        case .optionsOnly: "Choose an option above."
        case .keys: "Choose an option above or use the arrow keys."
        case .keysAndText: "Choose an option above, use the arrow keys, or type an answer."
        }
    }

    var canSendPrompt: Bool {
        !sendingPrompt && (!draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || (!answering && !attachments.isEmpty))
    }

    /// Runs while the screen is visible: watch, poll the core at 10 Hz, unwatch on cancel.
    func run() async {
        watch(route.terminalId)
        async let loaded: Void = loadMacDraft()
        while !Task.isCancelled {
            poll()
            try? await Task.sleep(for: .milliseconds(100))
        }
        watch(nil)
        await loaded
    }

    /// Text left unsent in the Mac's input box moves to the phone's field, unless the phone already has a draft.
    func loadMacDraft() async {
        while agent == nil, !Task.isCancelled {
            try? await Task.sleep(for: .milliseconds(100))
        }
        guard agent?.kind == "claude",
            let text = try? await core.agentDraft(machineId: route.machineId, terminalId: route.terminalId),
            !Task.isCancelled
        else { return }
        if text.isEmpty {
            macDraft = ""
        } else if draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty, attachments.isEmpty {
            draft = text
            macDraft = text
        }
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
        guard !refreshing else { return }
        refreshing = true
        defer { refreshing = false }
        do {
            ansi = try await core.agentRead(machineId: route.machineId, terminalId: route.terminalId, source: .recent).ansi
            notice = nil
        } catch {
            notice = Self.message(for: error)
        }
    }

    /// Text typed and files attached while the send is in flight stay for the next prompt.
    /// Paths on the Mac never have spaces, so they are set apart by single spaces.
    func sendPrompt() async {
        if answering {
            await sendAnswer()
            return
        }
        let sent = draft
        let files = attachments
        let typed = sent.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !sendingPrompt, !typed.isEmpty || !files.isEmpty else { return }
        let text = (files.map(\.path) + [typed].filter { !$0.isEmpty }).joined(separator: " ")
        sendingPrompt = true
        promptError = nil
        defer { sendingPrompt = false }
        do {
            try await core.prompt(machineId: route.machineId, terminalId: route.terminalId, text: text, expectedDraft: macDraft)
            if macDraft != nil { macDraft = "" }
            let sentIds = Set(files.map(\.id))
            attachments.removeAll { sentIds.contains($0.id) }
            if draft.hasPrefix(sent) {
                draft = String(draft.dropFirst(sent.count).drop(while: \.isWhitespace))
            }
        } catch {
            if case .DraftChanged(let current) = error as? CoreError { macDraft = current }
            promptError = Self.message(for: error)
        }
    }

    /// Typed into the prompt's own answer field and submitted with Enter; attachments stay for the next prompt.
    private func sendAnswer() async {
        let sent = draft
        let typed = sent.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !sendingPrompt, !typed.isEmpty else { return }
        sendingPrompt = true
        promptError = nil
        defer { sendingPrompt = false }
        do {
            try await core.typeText(machineId: route.machineId, terminalId: route.terminalId, text: typed)
            if draft.hasPrefix(sent) {
                draft = String(draft.dropFirst(sent.count).drop(while: \.isWhitespace))
            }
        } catch {
            promptError = Self.message(for: error)
        }
    }

    var attachmentSlots: Int { Attachment.maxPerPrompt - attachments.count }

    func remove(_ file: AttachedFile) {
        attachments.removeAll { $0.id == file.id }
    }

    @discardableResult
    func attach(name: String, load: @escaping @Sendable (UInt64) async throws -> Data) -> Task<Void, Never>? {
        attach([PendingAttachment(name: name, load: load)])
    }

    /// One batch at a time, uploaded in order; each uploaded file joins `attachments`.
    /// A file that fails is reported and the rest still go.
    @discardableResult
    func attach(_ items: [PendingAttachment]) -> Task<Void, Never>? {
        guard upload == nil, !items.isEmpty else { return nil }
        promptError = nil
        let slots = max(attachmentSlots, 0)
        var firstError: (any Error)? = items.count > slots ? AttachmentError.tooMany(limit: Attachment.maxPerPrompt) : nil
        let batch = Array(items.prefix(slots))
        guard !batch.isEmpty else {
            promptError = firstError.map(Self.message(for:))
            return nil
        }
        let core = core
        let machineId = route.machineId
        let limit = core.maxAttachmentBytes()
        let first = UUID()
        upload = AttachmentUpload(id: first, name: batch[0].name, count: batch.count)
        let task = Task {
            for (offset, item) in batch.enumerated() {
                if Task.isCancelled { break }
                let id = offset == 0 ? first : UUID()
                upload = AttachmentUpload(id: id, name: item.name, index: offset + 1, count: batch.count)
                do {
                    let data = try await item.load(limit)
                    try Attachment.check(data, limit: limit)
                    try Task.checkCancellation()
                    if upload?.id == id { upload?.total = UInt64(data.count) }
                    let progress = UploadProgressRelay { [weak self] sent, total in
                        Task { @MainActor in self?.uploaded(id, sent: sent, total: total) }
                    }
                    let path = try await core.uploadAttachment(machineId: machineId, name: item.name, data: data, progress: progress)
                    var file = AttachedFile(path: path, name: item.name)
                    if file.kind == .image { file.thumbnail = await Attachment.thumbnail(from: data) }
                    try Task.checkCancellation()
                    attachments.append(file)
                } catch {
                    if Task.isCancelled || error is CancellationError { break }
                    firstError = firstError ?? error
                }
            }
            if !Task.isCancelled, let firstError { promptError = Self.message(for: firstError) }
            if !Task.isCancelled { upload = nil }
        }
        uploadTask = task
        return task
    }

    func attachFailed(_ error: any Error) {
        promptError = Self.message(for: error)
    }

    /// Swift task cancellation does not reach Rust, so the core is told to abort the upload on the Mac.
    func cancelUpload() {
        guard upload != nil else { return }
        core.cancelUploads(machineId: route.machineId)
        uploadTask?.cancel()
        uploadTask = nil
        upload = nil
    }

    private func uploaded(_ id: UUID, sent: UInt64, total: UInt64) {
        guard upload?.id == id, let current = upload, sent >= current.sent else { return }
        upload?.sent = sent
        upload?.total = total
    }

    /// Keys go out in tap order: taps made while a send is in flight are batched into the next call.
    @discardableResult
    func tap(_ key: AgentKey) -> Task<Void, Never>? {
        guard acceptsKeys else { return nil }
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
            try await core.closeConfirmed(target, route: route)
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
        switch error as? CoreError {
        case .AgentBlocked:
            return "The agent is waiting for an approval. Answer it above or in the Approvals tab."
        case .DraftChanged(let current):
            let shown = current.count > 80 ? String(current.prefix(80)) + "…" : current
            return "The Mac's input box has unsent text: “\(shown)”. Send again to replace it."
        default:
            return describe(error)
        }
    }
}

extension AgentCore {
    /// Only for a target that `CloseConfirmation.confirm()` returned.
    func closeConfirmed(_ target: CloseTarget, route: AgentRoute) async throws {
        switch target {
        case .pane:
            try await closePane(machineId: route.machineId, terminalId: route.terminalId, confirm: true)
        case .workspace(let workspaceId):
            try await closeWorkspace(machineId: route.machineId, workspaceId: workspaceId, confirm: true)
        }
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
    static let strip: [AgentKey] = [.esc, .enter, .left, .up, .down, .right, .tab, .shiftTab, .ctrlC]

    var symbol: String {
        switch self {
        case .esc: "esc"
        case .enter: "⏎"
        case .up: "↑"
        case .down: "↓"
        case .left: "←"
        case .right: "→"
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
        case .left: "Left arrow"
        case .right: "Right arrow"
        case .tab: "Tab"
        case .shiftTab: "Shift Tab"
        case .ctrlC: "Control C"
        case .y: "Y"
        case .n: "N"
        }
    }
}
