import CollieCore
import Foundation
import Observation

/// What the pane runs now: its agent, a plain shell once the agent exited, or nothing.
enum PaneMode: Equatable {
    case agent, terminal, gone
}

@MainActor
@Observable
final class AgentModel {
    let route: AgentRoute
    private let core: any AgentCore
    private let unlocker: any TerminalUnlocker
    private let machineLabel: String?

    private(set) var agent: AgentSummary?
    private(set) var terminal: TerminalSummary?
    private(set) var terminalLocked = true
    private(set) var terminalsEnabled = false
    /// An agent ran in this pane while the screen was open.
    private(set) var hadAgent = false
    /// This phone did not pair with the terminal key it has now.
    private(set) var terminalKeyMissing = false
    private(set) var unlocking = false
    private var watchingShell = false
    private var lastMode: PaneMode?
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

    let dictation: DictationModel

    private let prefsFile: URL?
    var wrapLines: Bool {
        didSet {
            var prefs = DevicePrefs.load(from: prefsFile)
            prefs.wrapLines = wrapLines
            prefs.save(to: prefsFile)
        }
    }

    var fontSize: Double {
        didSet {
            var prefs = DevicePrefs.load(from: prefsFile)
            prefs.fontSize = fontSize
            prefs.save(to: prefsFile)
        }
    }

    private(set) var gestures: TerminalGestures

    var keepsKeyboard: Bool { DevicePrefs.load(from: prefsFile).keepKeyboard }

    var historyLines: UInt16 { DevicePrefs.load(from: prefsFile).historyLines }

    func reloadGestures() {
        gestures = DevicePrefs.load(from: prefsFile).gestures
    }

    // One chain per machine: a late unwatch from a popped screen must not land after
    // the next screen's watch on the same machine.
    private static var watchChains: [String: Task<Void, Never>] = [:]
    // A screen replaced in place stops after its successor on the machine has watched, so only
    // the latest watcher may unwatch.
    private static var watchers: [String: ObjectIdentifier] = [:]

    init(
        core: any AgentCore, route: AgentRoute, prefsFile: URL? = DevicePrefs.file,
        dictationEngine: any DictationEngine = SpeechDictationEngine(),
        unlocker: any TerminalUnlocker = SecureEnclaveUnlocker(), machineLabel: String? = nil
    ) {
        self.core = core
        self.route = route
        self.unlocker = unlocker
        self.machineLabel = machineLabel
        self.prefsFile = prefsFile
        let prefs = DevicePrefs.load(from: prefsFile)
        dictation = DictationModel(engine: dictationEngine, language: prefs.dictationLanguage, prefsFile: prefsFile)
        wrapLines = prefs.wrapLines
        fontSize = prefs.fontSize
        gestures = prefs.gestures
    }

    var mode: PaneMode? {
        if agent != nil { return .agent }
        if terminal != nil { return .terminal }
        return link == .connected ? .gone : nil
    }

    var isTerminal: Bool { mode == .terminal }

    var acceptsKeys: Bool { isTerminal ? !unlocking : blocked != .optionsOnly && blocked != .terminal }

    /// Typed text answers the blocking prompt instead of prompting the agent.
    var answering: Bool { !isTerminal && blocked == .keysAndText }

    /// What the screen says in place of the agent's prompt.
    var paneNotice: String? {
        switch mode {
        case .terminal?:
            if terminalKeyMissing {
                return unlocker.passcodeSet
                    ? "Pair this phone again to use terminals on this machine."
                    : "Set a passcode on this phone, then pair it again to use terminals on this machine."
            }
            guard terminalLocked else { return nil }
            return hadAgent ? "The agent exited. This pane is now a shell." : "This pane is a shell."
        case .gone?:
            if hadAgent && !terminalsEnabled {
                return "The agent exited. Terminals are off on this machine: set [terminals] enabled = true in collied.toml there and restart collied."
            }
            return "No longer running"
        case .agent?, nil:
            return nil
        }
    }

    var canUnlock: Bool { isTerminal && terminalLocked && !terminalKeyMissing && !unlocking }

    var blockedHint: String? {
        switch blocked {
        case nil, .terminal: nil
        case .optionsOnly: "Choose an option above."
        case .keys: "Choose an option above or use the arrow keys."
        case .keysAndText: "Choose an option above, use the arrow keys, or type an answer."
        }
    }

    var canSendPrompt: Bool {
        if isTerminal { return !sendingPrompt && !unlocking && !draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty }
        return !sendingPrompt && !dictation.isActive && (!draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || (!answering && !attachments.isEmpty))
    }

    /// Runs while the screen is visible: watch, poll the core, unwatch on cancel. collied pushes at about
    /// 4 Hz; a shell polls at 10 Hz so its echo does not wait up to another 250 ms.
    func run() async {
        Self.watchers[route.machineId] = ObjectIdentifier(self)
        watch(route.terminalId)
        var loading: Task<Void, Never>?
        while !Task.isCancelled {
            poll()
            if loading == nil, agent != nil {
                loading = Task { await loadMacDraft() }
            }
            try? await Task.sleep(for: .milliseconds(isTerminal ? 100 : 250))
        }
        if Self.watchers[route.machineId] == ObjectIdentifier(self) {
            Self.watchers[route.machineId] = nil
            watch(nil)
        }
        loading?.cancel()
        await loading?.value
    }

    /// Text left unsent in the Mac's input box moves to the phone's field, unless the phone already has a draft.
    func loadMacDraft() async {
        guard agent?.kind == "claude",
            let text = try? await core.agentDraft(machineId: route.machineId, terminalId: route.terminalId),
            !Task.isCancelled
        else { return }
        if text.isEmpty {
            macDraft = ""
        } else if draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty, attachments.isEmpty, !dictation.isActive {
            draft = text
            macDraft = text
        }
    }

    func poll() {
        guard let view = core.agentView(machineId: route.machineId, terminalId: route.terminalId, afterRevision: revision) else { return }
        if link != view.link { link = view.link }
        if linkError != view.lastError { linkError = view.lastError }
        if agent != view.agent { agent = view.agent }
        if terminal != view.terminal { terminal = view.terminal }
        if terminalLocked != view.terminalLocked { terminalLocked = view.terminalLocked }
        if terminalsEnabled != view.terminalsEnabled { terminalsEnabled = view.terminalsEnabled }
        if agent != nil, !hadAgent { hadAgent = true }
        if let output = view.output {
            ansi = output.ansi
            revision = view.outputRevision
        }
        followMode()
    }

    /// The shell is watched only while unlocked; an agent starting there takes the agent's watch back.
    /// Input typed or queued for the agent never reaches the shell after it, nor the reverse.
    private func followMode() {
        if let now = mode, now != lastMode {
            let previous = lastMode
            lastMode = now
            if previous != nil, now == .terminal || previous == .terminal { dropInput() }
            if now == .agent, previous == .terminal || previous == .gone {
                watchingShell = false
                watch(route.terminalId)
            }
        }
        if mode == .terminal, !terminalLocked, !watchingShell {
            watchingShell = true
            watchShell()
        } else if terminalLocked, watchingShell {
            watchingShell = false
        }
    }

    /// Face ID or the passcode for this terminal, never raised on its own: only from a tap on
    /// Unlock or an input. The reason names the workspace and the machine, never a title a
    /// program set.
    @discardableResult
    func unlock() async -> Bool {
        guard canUnlock else { return !terminalLocked && !terminalKeyMissing }
        unlocking = true
        defer { unlocking = false }
        let workspace = terminal?.workspaceLabel ?? "a workspace"
        let reason = "Open a terminal in \(workspace) on \(machineLabel ?? "the machine")"
        do {
            let message = try await core.terminalChallenge(machineId: route.machineId, terminalId: route.terminalId)
            guard let signature = await unlocker.sign(message, reason: reason) else { return false }
            try await core.terminalGrant(machineId: route.machineId, terminalId: route.terminalId, signature: signature)
            terminalLocked = false
            notice = nil
            followMode()
            return true
        } catch {
            if case .TerminalKeyMissing = error as? CoreError {
                terminalKeyMissing = true
            } else {
                notice = Self.message(for: error)
            }
            return false
        }
    }

    /// A shell is only read through its watch, under a grant.
    func refresh() async {
        guard !refreshing, !isTerminal else { return }
        refreshing = true
        defer { refreshing = false }
        do {
            ansi = try await core.agentRead(machineId: route.machineId, terminalId: route.terminalId, source: .recent, lines: historyLines).ansi
            notice = nil
        } catch {
            notice = Self.message(for: error)
        }
    }

    /// Text typed and files attached while the send is in flight stay for the next prompt.
    /// Paths on the Mac never have spaces, so they are set apart by single spaces.
    func sendPrompt() async {
        if isTerminal {
            await sendCommand()
            return
        }
        guard !dictation.isActive else { return }
        dictation.problem = nil
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

    /// One line, then Enter, into the shell; a locked terminal is unlocked first, in the same action.
    private func sendCommand() async {
        let sent = draft
        guard !sendingPrompt, !sent.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return }
        if sent.contains(where: \.isNewline) {
            promptError = "One command at a time"
            return
        }
        sendingPrompt = true
        promptError = nil
        defer { sendingPrompt = false }
        guard await unlock() else { return }
        do {
            try await core.terminalRun(machineId: route.machineId, terminalId: route.terminalId, text: sent)
            if draft.hasPrefix(sent) {
                draft = String(draft.dropFirst(sent.count))
            }
        } catch {
            if case .TerminalLocked = error as? CoreError { terminalLocked = true }
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

    private func dropInput() {
        dictation.cancel()
        cancelUpload()
        draft = ""
        macDraft = nil
        attachments = []
        queuedKeys.removeAll()
        promptError = nil
    }

    func paste(_ text: String?) {
        guard let text, !dictation.isActive else { return }
        draft += text
    }

    /// Dictation owns the field until it stops; a send in flight would trim the text it started from.
    @discardableResult
    func startDictation() -> Task<Void, Never>? {
        guard !sendingPrompt else { return nil }
        return dictation.start(appendingTo: draft) { [weak self] in self?.draft = $0 }
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
        let task = Task { [self] in
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
            if isTerminal, !(await unlock()) {
                queuedKeys.removeAll()
                return
            }
            let batch = Array(queuedKeys.prefix(16))
            queuedKeys.removeFirst(batch.count)
            do {
                if isTerminal {
                    try await core.terminalSendKeys(machineId: route.machineId, terminalId: route.terminalId, keys: batch)
                } else {
                    try await core.sendKeys(machineId: route.machineId, terminalId: route.terminalId, keys: batch)
                }
                notice = nil
            } catch {
                if case .TerminalLocked = error as? CoreError { terminalLocked = true }
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
        let machineId = route.machineId
        let previous = Self.watchChains[machineId]
        let core = core
        let lines = historyLines
        Self.watchChains[machineId] = Task {
            await previous?.value
            try? await core.watchAgent(machineId: machineId, terminalId: terminalId, lines: lines)
        }
    }

    private func watchShell() {
        let (machineId, terminalId) = (route.machineId, route.terminalId)
        let previous = Self.watchChains[machineId]
        let core = core
        let lines = historyLines
        Self.watchChains[machineId] = Task {
            await previous?.value
            try? await core.watchTerminal(machineId: machineId, terminalId: terminalId, lines: lines)
        }
    }

    static func message(for error: any Error) -> String {
        switch error as? CoreError {
        case .AgentBlocked:
            return "The agent is waiting for an approval. Answer it above or in the Approvals tab."
        case .DraftChanged(let current):
            let shown = current.count > 80 ? String(current.prefix(80)) + "…" : current
            return "The agent's input box has unsent text: “\(shown)”. Send again to replace it."
        case .TerminalLocked:
            return "The terminal locked. Unlock it again to continue."
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
    static let strip: [AgentKey] = [.esc, .left, .up, .down, .right, .tab, .shiftTab, .enter, .ctrlEnter]
    static let terminalStrip: [AgentKey] = [.esc, .tab, .ctrlC, .left, .up, .down, .right, .enter]

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
        case .ctrlEnter: "⌃⏎"
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
        case .ctrlEnter: "Control Return"
        case .y: "Y"
        case .n: "N"
        }
    }
}
