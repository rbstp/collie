import CollieCore
import Foundation
import Observation
import SwiftUI

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
    @ObservationIgnored private let now: () -> ContinuousClock.Instant
    @ObservationIgnored private let sleep: @Sendable (Duration) async throws -> Void
    @ObservationIgnored private let resends: PromptResends

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
    /// Whether the restored draft was typed into a shell, until the screen sees the pane's mode.
    private var restoredShell: Bool?
    private(set) var link: LinkPhase?
    private(set) var linkError: String?
    private(set) var ansi = ""
    private(set) var reflowed: String?
    private var screenBanner = false
    private var screenNotice: [NoticeOption] = []
    /// When the phone first saw the notice; Claude Code ignores a digit within 600 ms of showing it and
    /// leaves the digit in the input box.
    @ObservationIgnored private var noticeSince: ContinuousClock.Instant?
    private var noticeArmed = false
    private(set) var refreshing = false
    private var revision: UInt64 = 0

    let gitReview: GitReview
    let conversation = ConversationModel()
    var fileLink: FileLink?
    var draft = ""
    /// What the phone last saw in the Mac's input box: nil when unknown. A send replaces exactly this text.
    private(set) var macDraft: String?
    /// The Mac's box text loaded into the field, until the phone edits it; a prompt sent from the Mac clears it.
    @ObservationIgnored private var loadedMacDraft: String?
    var showsMacDraft: Bool { draft == loadedMacDraft }
    /// The agent's last prompt when the field was loaded or the box last checked: only a new one means the Mac sent.
    @ObservationIgnored private var seenPrompt: String?
    @ObservationIgnored private(set) var sentMacDraftCheck: Task<Void, Never>?
    private(set) var sendingPrompt = false
    var retrying: Bool { sendingPrompt && resends.entries[route].map { $0.retrying && !$0.stopped } == true }
    /// The prompt this model sent or took over, while it is in flight.
    @ObservationIgnored private var resend: PromptResends.Entry?
    /// The prompt ended with another screen, or none, which keeps the draft instead.
    @ObservationIgnored private var draftElsewhere = false
    /// The Mac's input box holds text the mirror did not put there; it waits for the next send.
    private var mirrorPaused = false
    /// The draft as the keyboard last left it; dictation, paste and restore never reach the Mac.
    @ObservationIgnored private var typedDraft: String?
    @ObservationIgnored private var mirrorDelay: Task<Void, Never>?
    @ObservationIgnored private var mirroring: Task<Void, Never>?
    /// The last command this screen asked collied to paste; only that is replaced unseen.
    @ObservationIgnored private var mirrored = ""
    @ObservationIgnored private var codexFooter: (ansi: String, open: Bool)?
    /// ↑ or ↓ moved Claude Code's highlight off what the box holds; ⇥ takes it.
    @ObservationIgnored private var menuMoved = false
    private(set) var promptError: String?
    private let uploads = AttachmentUploads()
    var upload: AttachmentUpload? { uploads.current }
    private(set) var attachments: [AttachedFile] = []

    var blocked: BlockedInput?
    private(set) var notice: String?
    private(set) var keyTaps = 0
    /// Shift or Control waiting for the next key; at most one, so a combination is never more than two keys.
    var modifier: KeyModifier?
    private(set) var quotes = 0
    private enum Input {
        case key(AgentKey)
        case answer(String, CheckedContinuation<Void, any Error>)
    }
    private var queuedInput: [Input] = []
    private var sendingInput = false

    var close = CloseConfirmation()
    private(set) var closed = false
    private(set) var archiveMessage: String?
    private(set) var archiving = false

    let dictation: DictationModel

    private let prefsFile: URL?
    private let draftsFile: URL?
    var screen: String { wrapLines ? reflowed ?? ansi : ansi }

    var wrapLines: Bool {
        didSet { DevicePrefs.update(in: prefsFile) { $0.wrapLines = wrapLines } }
    }

    var keysShown: Bool {
        didSet {
            DevicePrefs.update(in: prefsFile) { $0.keysShown = keysShown }
            if !keysShown { modifier = nil }
        }
    }

    var fontSize: Double

    func saveFontSize() {
        DevicePrefs.update(in: prefsFile) { $0.fontSize = fontSize }
    }

    private(set) var gestures: TerminalGestures

    var savesPreferences: Bool { prefsFile != nil }

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
        core: any AgentCore, route: AgentRoute, prefsFile: URL? = DevicePrefs.file, draftsFile: URL? = nil, viewedFile: URL? = nil,
        dictationEngine: any DictationEngine = SpeechDictationEngine(),
        unlocker: any TerminalUnlocker = SecureEnclaveUnlocker(), machineLabel: String? = nil,
        now: @escaping () -> ContinuousClock.Instant = { .now },
        sleep: @escaping @Sendable (Duration) async throws -> Void = { try await Task.sleep(for: $0) },
        resends: PromptResends = .shared
    ) {
        self.core = core
        self.now = now
        self.sleep = sleep
        self.resends = resends
        self.route = route
        self.unlocker = unlocker
        self.machineLabel = machineLabel
        self.prefsFile = prefsFile
        self.draftsFile = draftsFile
        gitReview = GitReview(viewedFile: viewedFile)
        let prefs = DevicePrefs.load(from: prefsFile)
        dictation = DictationModel(engine: dictationEngine, language: prefs.dictationLanguage, prefsFile: prefsFile)
        wrapLines = prefs.wrapLines
        keysShown = prefs.keysShown
        fontSize = prefs.fontSize
        gestures = prefs.gestures
        if let saved = AgentDrafts.load(from: draftsFile).drafts[route] {
            draft = saved.text
            attachments = saved.attachments.filter { $0.uploaded.timeIntervalSinceNow > -Attachment.keptOnMachine }
            restoredShell = saved.shell
        }
        promptError = resends.takeUnseen(route)
    }

    /// A send in flight saves what stays once it succeeds, so a screen opened meanwhile never offers
    /// the sent prompt again, and saves again when it ends.
    func saveDraft() {
        guard !sendingPrompt else { return }
        saveDraft(draft, attachments)
    }

    /// Text loaded from the Mac's input box is not kept: it loads again when the screen opens.
    private func saveDraft(_ text: String, _ files: [AttachedFile]) {
        guard let draftsFile else { return }
        var saved = AgentDrafts.load(from: draftsFile)
        let typed = !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty && text != macDraft
        let shell = restoredShell ?? (lastMode == .terminal)
        let next = typed || !files.isEmpty ? AgentDrafts.Draft(text: typed ? text : "", attachments: files, shell: shell) : nil
        guard next != nil || saved.drafts[route] != nil else { return }
        saved.drafts[route] = next
        saved.save(to: draftsFile)
    }

    var mode: PaneMode? {
        if agent != nil { return .agent }
        if terminal != nil { return .terminal }
        return link == .connected ? .gone : nil
    }

    var isTerminal: Bool { mode == .terminal }

    /// Claude Code's transcript is scrolled up on the Mac; collied refuses keys until it jumps back down.
    var jumpBanner: Bool { mode == .agent && agent?.kind == "claude" && screenBanner }

    /// A Claude Code notice's options, in screen order; collied sends the digit only while it shows.
    var noticeOptions: [NoticeOption] {
        mode == .agent && agent?.kind == "claude" && agent?.status != .blocked && blocked == nil && !jumpBanner && noticeArmed ? screenNotice : []
    }

    /// A slash command shows in Claude Code's input box, so Enter would run its highlighted command.
    var commandShown: Bool { mode == .agent && blocked == nil && agent?.status != .blocked && macDraft?.hasPrefix("/") == true }

    var acceptsKeys: Bool {
        isTerminal ? !unlocking : answering || !jumpBanner
    }

    func accepts(_ key: AgentKey) -> Bool {
        if key == .shiftLeft { return agent?.kind == "codex" }
        return acceptsKeys
    }

    func canTap(_ key: AgentKey) -> Bool {
        accepts(key) && !(commandShown && (key == .enter || key == .ctrlEnter))
    }

    /// With a modifier selected, only keys it combines with stay on.
    func canPress(_ key: AgentKey) -> Bool {
        guard let modifier else { return canTap(key) }
        return key.with(modifier).map(canTap) ?? false
    }

    /// The pane's kind as shortcuts name it: "terminal" for a shell.
    var shortcutKind: String? { isTerminal ? "terminal" : agent?.kind }

    var codexQuestionQueued: Bool {
        guard agent?.kind == "codex" else { return false }
        let tail = ansi.split(omittingEmptySubsequences: false, whereSeparator: \.isNewline).suffix(16)
        return tail.contains { $0.contains("Queued follow-up inputs") }
            && tail.contains { $0.contains("⇧←") && $0.contains("to answer") }
            && !codexQuestionOpen
    }

    var codexQuestionOpen: Bool {
        guard agent?.kind == "codex" else { return false }
        if let codexFooter, codexFooter.ansi == ansi { return codexFooter.open }
        let lines = ansi.split(omittingEmptySubsequences: false, whereSeparator: \.isNewline)
        let footer = lines.reversed().lazy
            .map { $0.replacingOccurrences(of: "\u{1B}\\[[0-9;]*[mC]", with: "", options: .regularExpression).trimmingCharacters(in: .whitespaces) }
            .first { !$0.isEmpty }
        let open = footer?.contains("enter submit") == true && footer?.contains("main prompt") == true
            && lines.suffix(25).contains { $0.contains("›") || $0.contains("Queued follow-up inputs") }
        codexFooter = (ansi, open)
        return open
    }

    var answering: Bool { !isTerminal && (blocked != nil || agent?.status == .blocked || codexQuestionOpen) }

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
        answering ? "Use the terminal buttons or type an answer." : nil
    }

    var canSendPrompt: Bool {
        if isTerminal { return !sendingPrompt && !unlocking && !draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty }
        return !sendingPrompt && !dictation.isActive
            && (!draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || (!answering && !attachments.isEmpty))
    }

    /// Runs while the screen is visible: watch, poll the core, unwatch on cancel. collied pushes at about
    /// 4 Hz; a shell polls at 10 Hz so its echo does not wait up to another 250 ms.
    func history(before: String?, after: String?, query: String?) async throws -> HistoryPage {
        try await core.agentHistory(machineId: route.machineId, terminalId: route.terminalId, before: before, after: after, query: query)
    }

    func run() async {
        if let resend, resends.entries[route] === resend, resend.viewer == nil { resend.viewer = self }
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
        if resend?.stopped == false { resends.leave(route, self) }
        if Self.watchers[route.machineId] == ObjectIdentifier(self) {
            Self.watchers[route.machineId] = nil
            watch(nil)
        }
        loading?.cancel()
        await loading?.value
    }

    /// Text left unsent in the Mac's input box moves to the phone's field, unless the phone already has a draft.
    func loadMacDraft() async {
        let prompt = agent?.lastPrompt
        guard agent?.kind == "claude",
            let text = try? await core.agentDraft(machineId: route.machineId, terminalId: route.terminalId),
            !Task.isCancelled
        else { return }
        mirrorPaused = false
        if text.isEmpty {
            macDraft = ""
        } else if draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty, attachments.isEmpty, !dictation.isActive {
            draft = text
            macDraft = text
            loadedMacDraft = text
            seenPrompt = prompt
        } else if text == Self.boxToken(text), Self.slashToken(draft) == text {
            macDraft = text
        }
    }

    /// Keyboard typing in the prompt field: only this mirrors the draft's slash command to the Mac.
    func typed(_ text: String) {
        if text != draft {
            loadedMacDraft = nil
            stopRetry()
        }
        draft = text
        typedDraft = text
        mirrorDelay?.cancel()
        mirrorDelay = Task { [weak self] in
            try? await Task.sleep(for: .milliseconds(150))
            guard !Task.isCancelled else { return }
            await self?.flushMirror()
        }
    }

    /// Runs the mirror now, or waits for the one in flight, which follows the latest draft.
    func flushMirror() async {
        mirrorDelay?.cancel()
        mirrorDelay = nil
        let task = mirroring ?? Task { await runMirror() }
        mirroring = task
        await task.value
    }

    private var mirrors: Bool {
        mode == .agent && agent?.kind == "claude" && agent?.status != .blocked && blocked == nil && !jumpBanner
            && !dictation.isActive && !sendingPrompt && !mirrorPaused
    }

    /// One call at a time until the Mac's box holds the draft's command token, or nothing once the
    /// draft no longer starts with one. An unknown box is never written.
    private func runMirror() async {
        while mirrors, draft == typedDraft, let box = macDraft.flatMap(Self.boxToken), Self.slashToken(draft) != box {
            let target = Self.slashToken(draft)
            if !target.isEmpty { mirrored = target }
            do {
                try await core.slashDraft(machineId: route.machineId, terminalId: route.terminalId, command: target, expectedDraft: macDraft)
                macDraft = target
                mirrored = target
                menuMoved = false
            } catch {
                guard case .DraftChanged(let current) = error as? CoreError else { break }
                macDraft = current
                if current != mirrored {
                    mirrorPaused = true
                    promptError = Self.message(for: error)
                }
            }
        }
        mirroring = nil
    }

    /// The draft's leading `/` and what follows up to the first whitespace, if collied would paste it; else "".
    static func slashToken(_ text: String) -> String {
        let token = text.prefix { !$0.isWhitespace }
        let name = token.dropFirst()
        guard token.first == "/", name.count <= 63,
            name.unicodeScalars.allSatisfy({ $0.isASCII && (CharacterSet.alphanumerics.contains($0) || "_:.-".unicodeScalars.contains($0)) })
        else { return "" }
        return String(token)
    }

    /// The command token in the Mac's box, "" when it is empty, nil when it holds other text.
    static func boxToken(_ text: String) -> String? {
        if text.isEmpty { return "" }
        guard text.hasPrefix("/") else { return nil }
        return String(text.prefix { !$0.isWhitespace })
    }

    /// A command picked on the Mac with Tab replaces the phone's; Tab's trailing space
    /// keeps what is typed next out of the command.
    private func adopt(_ text: String) {
        macDraft = text
        guard let box = Self.boxToken(text) else {
            mirrorPaused = true
            return
        }
        let mine = Self.slashToken(draft)
        guard !box.isEmpty, draft.hasPrefix("/"), box != mine else { return }
        let rest = draft.dropFirst(mine.count)
        draft = box + (rest.isEmpty ? " " : rest)
    }

    func poll() {
        adoptResend()
        if let resend, draft != resend.sent { stopRetry() }
        guard let view = core.agentView(machineId: route.machineId, terminalId: route.terminalId, afterRevision: revision) else { return }
        if link != view.link {
            link = view.link
            if link == .connected { resends.wake(route) }
        }
        if linkError != view.lastError { linkError = view.lastError }
        if agent != view.agent {
            if loadedMacDraft != nil, let prompt = view.agent?.lastPrompt, prompt != seenPrompt {
                seenPrompt = prompt
                dropSentMacDraft()
            }
            agent = view.agent
        }
        if terminal != view.terminal { terminal = view.terminal }
        if terminalLocked != view.terminalLocked { terminalLocked = view.terminalLocked }
        if terminalsEnabled != view.terminalsEnabled { terminalsEnabled = view.terminalsEnabled }
        if agent != nil, !hadAgent { hadAgent = true }
        if let output = view.output {
            ansi = output.ansi
            reflowed = output.reflowed
            screenBanner = output.jumpBanner
            show(notice: output.notice)
            revision = view.outputRevision
        }
        armNotice()
        followMode()
    }

    /// collied's last prompt is human typed only, so a background turn or text deleted on the Mac never gets here;
    /// a nil last prompt (transcript unreadable for a moment) is ignored. The Mac's box reads empty once it sent.
    private func dropSentMacDraft() {
        sentMacDraftCheck = Task {
            guard let box = try? await core.agentDraft(machineId: route.machineId, terminalId: route.terminalId), box.isEmpty,
                let loaded = loadedMacDraft, draft == loaded, !sendingPrompt
            else { return }
            draft = ""
            macDraft = ""
            loadedMacDraft = nil
            saveDraft()
        }
    }

    private func show(notice options: [NoticeOption]) {
        if options.isEmpty {
            noticeSince = nil
        } else if noticeSince == nil || options != screenNotice {
            noticeSince = now()
        }
        screenNotice = options
        armNotice()
    }

    private func armNotice() {
        let armed = noticeSince.map { now() - $0 >= .milliseconds(600) } ?? false
        if noticeArmed != armed { noticeArmed = armed }
    }

    /// The shell is watched only while unlocked; an agent starting there takes the agent's watch back.
    /// Input typed or queued for the agent never reaches the shell after it, nor the reverse.
    private func followMode() {
        if let now = mode, now != lastMode {
            let previous = lastMode
            lastMode = now
            let before = previous ?? restoredShell.map { $0 ? .terminal : .agent }
            restoredShell = nil
            if let before, before != now, now == .terminal || before == .terminal { dropInput() }
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
            let read = try await core.agentRead(machineId: route.machineId, terminalId: route.terminalId, source: .recent, lines: historyLines)
            ansi = read.ansi
            reflowed = read.reflowed
            screenBanner = read.jumpBanner
            show(notice: read.notice)
            notice = nil
        } catch {
            notice = Self.message(for: error)
        }
    }

    /// Text typed and files attached while the send is in flight stay for the next prompt.
    /// Paths on the Mac never have spaces, so they are set apart by single spaces.
    func sendPrompt() async {
        defer {
            if draftElsewhere { draftElsewhere = false } else { saveDraft() }
        }
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
        mirrorDelay?.cancel()
        await mirroring?.value
        guard !sendingPrompt else { return }
        if menuMoved, commandShown {
            promptError = "Press ⇥ to take the highlighted command, then send."
            return
        }
        sendingPrompt = true
        promptError = nil
        _ = resends.takeUnseen(route)
        defer { sendingPrompt = false }
        saveDraft("", [])
        do {
            try await prompt(text, sent: sent, files: files)
            finish(sent, files: files, error: nil)
        } catch {
            finish(sent, files: files, error: error)
        }
    }

    private func finish(_ sent: String, files: [AttachedFile], error: (any Error)?) {
        guard let error else {
            if macDraft != nil { macDraft = "" }
            mirrorPaused = false
            mirrored = ""
            menuMoved = false
            loadedMacDraft = nil
            let sentIds = Set(files.map(\.id))
            attachments.removeAll { sentIds.contains($0.id) }
            if draft.hasPrefix(sent) {
                draft = String(draft.dropFirst(sent.count).drop(while: \.isWhitespace))
            }
            return
        }
        if isTerminal { return }
        if case .DraftChanged(let current) = error as? CoreError {
            let mine = Self.slashToken(draft)
            if !mine.isEmpty, Self.boxToken(current)?.hasPrefix(mine) == true { adopt(current) } else { macDraft = current }
        }
        if case .AgentBlocked = error as? CoreError, codexQuestionQueued {
            promptError = "Tap ⇧← to view and answer the pending question."
        } else {
            promptError = Self.promptMessage(for: error)
        }
    }

    static func promptMessage(for error: any Error) -> String {
        PromptRetry.retries(error) ? "May not have been sent: machine unreachable" : message(for: error)
    }

    /// A prompt still in flight for this agent, from a screen that did not take it over, ends first, so
    /// prompts arrive in order. The last prompt counts only when the link was up, so it is not stale.
    private func prompt(_ text: String, sent: String, files: [AttachedFile]) async throws {
        while true {
            if let pending = resends.entries[route] {
                pending.viewer = self
                resends.stop(route)
                _ = await pending.outcome.value
                continue
            }
            let linked = link == .connected
            let send = PromptResends.Send(
                core: core, route: route, text: text, expected: macDraft, before: linked ? agent?.lastPrompt : nil,
                draftsFile: draftsFile, now: now, sleep: sleep
            )
            guard let entry = resends.start(send, sent: sent, files: files, linked: linked, viewer: self) else { continue }
            resend = entry
            let outcome = await entry.outcome.value
            resend = nil
            draftElsewhere = entry.viewer !== self
            if let outcome { throw outcome }
            return
        }
    }

    /// A screen that opens while its agent's prompt is being resent shows it, as if it never left.
    private func adoptResend() {
        guard !sendingPrompt, let entry = resends.entries[route], entry.viewer == nil, !entry.stopped, attachments.isEmpty,
            draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || draft == loadedMacDraft
        else { return }
        entry.viewer = self
        resend = entry
        loadedMacDraft = nil
        draft = entry.sent
        attachments = entry.files
        sendingPrompt = true
        Task {
            let outcome = await entry.outcome.value
            resend = nil
            if entry.viewer === self {
                finish(entry.sent, files: entry.files, error: outcome)
                sendingPrompt = false
                saveDraft()
            } else {
                sendingPrompt = false
            }
        }
    }

    func stopRetry() {
        if let resend, resends.entries[route] === resend { resends.stop(route) }
    }

    /// One line, then Enter, into the shell; a locked terminal is unlocked first, in the same action.
    /// A shortcut's command leaves the draft alone.
    private func sendCommand(_ shortcut: String? = nil) async {
        let sent = shortcut ?? draft
        guard !sendingPrompt, !sent.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return }
        if sent.contains(where: \.isNewline) {
            promptError = "One command at a time"
            return
        }
        sendingPrompt = true
        promptError = nil
        defer { sendingPrompt = false }
        if shortcut == nil { saveDraft("", attachments) }
        guard await unlock() else { return }
        do {
            try await core.terminalRun(machineId: route.machineId, terminalId: route.terminalId, text: sent)
            if shortcut == nil, draft.hasPrefix(sent) {
                draft = String(draft.dropFirst(sent.count))
            }
        } catch {
            if case .TerminalLocked = error as? CoreError { terminalLocked = true }
            promptError = Self.message(for: error)
        }
    }

    private func sendAnswer() async {
        let sent = draft
        let typed = sent.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !sendingPrompt, !typed.isEmpty else { return }
        sendingPrompt = true
        promptError = nil
        defer { sendingPrompt = false }
        saveDraft("", attachments)
        do {
            try await typeAnswer(typed)
            if draft.hasPrefix(sent) {
                draft = String(draft.dropFirst(sent.count).drop(while: \.isWhitespace))
            }
        } catch {
            promptError = Self.message(for: error)
        }
    }

    func typeAnswer(_ text: String) async throws {
        try await withCheckedThrowingContinuation { (continuation: CheckedContinuation<Void, any Error>) in
            enqueue(.answer(text, continuation))
        }
    }

    private func dropInput() {
        stopRetry()
        dictation.cancel()
        cancelUpload()
        draft = ""
        macDraft = nil
        attachments = []
        cancelInput(CoreError.AgentNotReady)
        promptError = nil
    }

    func gitChanges(root: String? = nil, branch: Bool = false) async throws -> GitChanges {
        try await core.agentChanges(machineId: route.machineId, terminalId: route.terminalId, root: root, branch: branch)
    }

    func gitDiff(root: String, file: GitFile) async throws -> GitDiff {
        try await core.agentDiff(machineId: route.machineId, terminalId: route.terminalId,
                                 root: root, path: file.path, section: file.section)
    }

    func openFile(_ path: String) async throws -> OpenedFile {
        try await core.openFile(machineId: route.machineId, terminalId: route.terminalId, path: path)
    }

    func cancelFileOpen() {
        core.cancelDownloads(machineId: route.machineId)
    }

    /// Text sent apart from the draft: review feedback and text shortcuts.
    var canSendText: Bool {
        agent != nil && !isTerminal && !answering && !sendingPrompt && !dictation.isActive
            && blocked == nil && agent?.status != .blocked && !codexQuestionQueued && !jumpBanner
    }

    func sendGitFeedback(_ text: String) async -> String? {
        guard canSendText else { return "The agent cannot accept review feedback right now." }
        guard !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return "Write feedback before sending." }
        guard text.utf8.count <= 32 * 1024 else { return "Feedback is too long. Shorten the comments or select less text." }
        do {
            return try await sendAlone(text) ? nil : "The agent cannot accept review feedback right now."
        } catch {
            if case .DraftChanged = error as? CoreError {
                return "The agent’s input box has unsent text. Send or clear it in the session before sending this feedback."
            }
            return Self.message(for: error)
        }
    }

    /// With `expected_draft` "", collied never replaces unsent text in a Claude Code input box. False when it can no longer send.
    private func sendAlone(_ text: String) async throws -> Bool {
        mirrorDelay?.cancel()
        await mirroring?.value
        guard canSendText else { return false }
        sendingPrompt = true
        loadedMacDraft = nil
        defer { sendingPrompt = false; saveDraft() }
        try await core.prompt(machineId: route.machineId, terminalId: route.terminalId, text: text, expectedDraft: "")
        if macDraft != nil { macDraft = "" }
        mirrored = ""
        menuMoved = false
        return true
    }

    private static let tooLong = "The prompt would be longer than 32 KiB."

    @discardableResult
    func insert(_ shortcut: PromptShortcut) -> Bool {
        guard !isTerminal, !answering, !sendingPrompt, !dictation.isActive else { return false }
        let next = shortcut.appending(to: draft)
        guard next.utf8.count <= 32 * 1024 else {
            promptError = Self.tooLong
            return false
        }
        if promptError == Self.tooLong { promptError = nil }
        loadedMacDraft = nil
        draft = next
        return true
    }

    func quote(_ text: String) {
        guard !text.isEmpty, insert(PromptShortcut(text: CodeFence.block(text) + "\n")) else { return }
        quotes += 1
    }

    func paste(_ text: String?) {
        guard let text, !dictation.isActive else { return }
        loadedMacDraft = nil
        stopRetry()
        draft += text
    }

    /// Dictation owns the field until it stops; a send in flight would trim the text it started from.
    @discardableResult
    func startDictation(at selection: TextSelection? = nil) -> Task<Void, Never>? {
        guard !sendingPrompt else { return nil }
        loadedMacDraft = nil
        return dictation.start(in: draft, at: selection) { [weak self] in self?.draft = $0 }
    }

    var attachmentSlots: Int { Attachment.maxPerPrompt - attachments.count }

    func remove(_ file: AttachedFile) {
        attachments.removeAll { $0.id == file.id }
    }

    @discardableResult
    func attach(name: String, load: @escaping @Sendable (UInt64) async throws -> Data) -> Task<Void, Never>? {
        attach([PendingAttachment(name: name, load: load)])
    }

    @discardableResult
    func attach(_ items: [PendingAttachment]) -> Task<Void, Never>? {
        guard upload == nil, !items.isEmpty else { return nil }
        promptError = nil
        return uploads.start(items, slots: attachmentSlots, core: core, machineId: route.machineId) { file in
            self.attachments.append(file)
        } failed: { message in
            self.promptError = message
        }
    }

    func attachFailed(_ error: any Error) {
        promptError = Self.message(for: error)
    }

    /// Swift task cancellation does not reach Rust, so the core is told to abort the upload on the Mac.
    func cancelUpload() {
        guard upload != nil else { return }
        core.cancelUploads(machineId: route.machineId)
        uploads.cancel()
    }

    @discardableResult
    func tap(_ key: AgentKey) -> Task<Void, Never>? {
        guard canTap(key) else { return nil }
        keyTaps += 1
        return enqueue(.key(key))
    }

    func select(_ modifier: KeyModifier) {
        self.modifier = self.modifier == modifier ? nil : modifier
    }

    /// A key from the strip: with a modifier selected, the two go out together and the modifier clears.
    @discardableResult
    func press(_ key: AgentKey) -> Task<Void, Never>? {
        guard let modifier else { return tap(key) }
        self.modifier = nil
        return key.with(modifier).flatMap(tap)
    }

    /// A selected modifier applies to the next edit only. With Control, a listed letter typed on the
    /// keyboard goes out as Control and that letter instead of into the field; false for anything else.
    func controlTyped(from old: String, to new: String) -> Bool {
        guard let selected = modifier, new != old else { return false }
        modifier = nil
        guard selected == .control, new.count == old.count + 1 else { return false }
        let at = zip(old, new).prefix { $0 == $1 }.count
        var rest = new
        let letter = rest.remove(at: rest.index(rest.startIndex, offsetBy: at))
        guard rest == old, let key = AgentKey.control(letter), canTap(key) else { return false }
        tap(key)
        return true
    }

    func canRun(_ shortcut: StripShortcut) -> Bool {
        guard shortcut.applies(to: shortcutKind) else { return false }
        if let key = shortcut.agentKey { return canTap(key) }
        return isTerminal ? !sendingPrompt && !unlocking : canSendText
    }

    /// A text shortcut goes out on its own, as a command in a shell; collied refuses it while a
    /// Claude Code input box holds unsent text.
    @discardableResult
    func run(_ shortcut: StripShortcut) -> Task<Void, Never>? {
        modifier = nil
        guard canRun(shortcut) else { return nil }
        if let key = shortcut.agentKey { return tap(key) }
        if isTerminal { return Task { await sendCommand(shortcut.text) } }
        promptError = nil
        return Task {
            do {
                _ = try await sendAlone(shortcut.text)
            } catch {
                if case .DraftChanged = error as? CoreError {
                    promptError = "The agent’s input box has unsent text. Send or clear it, then tap the shortcut again."
                } else {
                    promptError = Self.message(for: error)
                }
            }
        }
    }

    @discardableResult
    private func enqueue(_ input: Input) -> Task<Void, Never>? {
        queuedInput.append(input)
        guard !sendingInput else { return nil }
        sendingInput = true
        return Task { await drainInput() }
    }

    private func cancelInput(_ error: any Error) {
        for case let .answer(_, continuation) in queuedInput {
            continuation.resume(throwing: error)
        }
        queuedInput.removeAll()
    }

    private func drainInput() async {
        defer { sendingInput = false }
        while let input = queuedInput.first {
            if case let .answer(text, continuation) = input {
                queuedInput.removeFirst()
                do {
                    try await core.typeText(machineId: route.machineId, terminalId: route.terminalId, text: text)
                    continuation.resume()
                } catch {
                    continuation.resume(throwing: error)
                    cancelInput(error)
                }
                continue
            }
            if isTerminal, !(await unlock()) {
                cancelInput(CoreError.TerminalLocked)
                return
            }
            if !isTerminal {
                await flushMirror()
            }
            guard case .key? = queuedInput.first else { continue }
            let command = commandShown
            var batch: [AgentKey] = []
            while batch.count < (codexQuestionOpen ? 1 : 16), case let .key(key)? = queuedInput.first {
                batch.append(key)
                queuedInput.removeFirst()
            }
            if command { batch.removeAll { $0 == .enter || $0 == .ctrlEnter } }
            if batch.isEmpty { continue }
            do {
                if isTerminal {
                    try await core.terminalSendKeys(machineId: route.machineId, terminalId: route.terminalId, keys: batch)
                } else {
                    try await core.sendKeys(machineId: route.machineId, terminalId: route.terminalId, keys: batch)
                }
                notice = nil
                if command, let last = batch.last(where: { $0 == .up || $0 == .down || $0 == .tab }) {
                    menuMoved = last != .tab
                }
                if command {
                    try? await Task.sleep(for: .milliseconds(150))
                    if let text = try? await core.agentDraft(machineId: route.machineId, terminalId: route.terminalId) { adopt(text) }
                }
                if batch.contains(.shiftLeft) { await refresh() }
            } catch {
                if case .TerminalLocked = error as? CoreError { terminalLocked = true }
                cancelInput(error)
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

    func watchPullRequest(everyMinutes: UInt32?) async {
        do {
            try await core.watchPullRequest(machineId: route.machineId, terminalId: route.terminalId, everyMinutes: everyMinutes)
            notice = nil
        } catch {
            notice = Self.message(for: error)
        }
    }

    func jumpToBottom() async {
        screenBanner = false
        do {
            try await core.scrollBottom(machineId: route.machineId, terminalId: route.terminalId)
            notice = nil
        } catch {
            screenBanner = true
            notice = Self.message(for: error)
        }
    }

    func answerNotice(_ option: NoticeOption) async {
        let shown = screenNotice
        let seen = revision
        screenNotice = []
        do {
            try await core.answerNotice(machineId: route.machineId, terminalId: route.terminalId, digit: option.digit, label: option.label)
            notice = nil
        } catch {
            if revision == seen { screenNotice = shown }
            notice = Self.message(for: error)
        }
    }

    /// Only reachable after the confirmation; the core is called with `confirm: true`.
    func performClose() async {
        guard !archiving, let target = close.confirm() else { return }
        archiving = target == .archive
        defer { archiving = false }
        do {
            if let message = try await core.closeConfirmed(target, route: route) {
                archiveMessage = message
            } else {
                closed = true
            }
        } catch {
            notice = Self.message(for: error)
        }
    }

    func finishArchive() {
        archiveMessage = nil
        closed = true
    }

    private func watch(_ terminalId: String?) {
        chain { core, machineId, lines in try await core.watchAgent(machineId: machineId, terminalId: terminalId, lines: lines) }
    }

    private func watchShell() {
        let terminalId = route.terminalId
        chain { core, machineId, lines in try await core.watchTerminal(machineId: machineId, terminalId: terminalId, lines: lines) }
    }

    private func chain(_ call: @escaping @Sendable (any AgentCore, String, UInt16) async throws -> Void) {
        let machineId = route.machineId
        let previous = Self.watchChains[machineId]
        let core = core
        let lines = historyLines
        Self.watchChains[machineId] = Task {
            await previous?.value
            try? await call(core, machineId, lines)
        }
    }

    static func message(for error: any Error) -> String {
        switch error as? CoreError {
        case .AgentBlocked:
            return "The agent is waiting for an approval. Use the terminal buttons or type an answer."
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
    func closeConfirmed(_ target: CloseTarget, route: AgentRoute) async throws -> String? {
        switch target {
        case .pane:
            try await closePane(machineId: route.machineId, terminalId: route.terminalId, confirm: true)
            return nil
        case .workspace(let workspaceId):
            try await closeWorkspace(machineId: route.machineId, workspaceId: workspaceId, confirm: true)
            return nil
        case .archive:
            return try await archiveTask(machineId: route.machineId, terminalId: route.terminalId, confirm: true)
        }
    }
}

enum CloseTarget: Equatable {
    case pane
    case workspace(id: String)
    case archive
}

/// Destructive closes need an explicit confirmation before the core sees `confirm: true`.
struct CloseConfirmation: Equatable {
    enum Step: Equatable {
        case idle
        case asking(CloseTarget)
        case confirmed(CloseTarget)
    }

    private(set) var step = Step.idle

    var target: CloseTarget? {
        switch step {
        case .idle: nil
        case .asking(let target), .confirmed(let target): target
        }
    }

    mutating func begin(_ target: CloseTarget) {
        step = .asking(target)
    }

    mutating func advance() {
        if case .asking(let target) = step { step = .confirmed(target) }
    }

    mutating func cancel() {
        step = .idle
    }

    mutating func confirm() -> CloseTarget? {
        guard case .confirmed(let target) = step else { return nil }
        step = .idle
        return target
    }
}

enum KeyModifier: CaseIterable {
    case shift, control

    var symbol: String {
        switch self {
        case .shift: "⇧"
        case .control: "⌃"
        }
    }

    var accessibilityName: String {
        switch self {
        case .shift: "Shift"
        case .control: "Control"
        }
    }
}

extension AgentKey {
    static let strip: [AgentKey] = [.esc, .left, .up, .down, .right, .tab, .backspace, .enter]
    static let combos: [AgentKey] = [.shiftTab, .ctrlEnter, .shiftLeft, .ctrlC]
    static let controlLetters: [AgentKey] = [.ctrlA, .ctrlB, .ctrlC, .ctrlE, .ctrlK, .ctrlL, .ctrlU, .ctrlW]
    static let all: [AgentKey] = strip + combos.filter { $0 != .ctrlC } + controlLetters

    /// The combination with a modifier; nil when collied has no such key.
    func with(_ modifier: KeyModifier) -> AgentKey? {
        switch (modifier, self) {
        case (.shift, .tab): .shiftTab
        case (.shift, .left): .shiftLeft
        case (.control, .enter): .ctrlEnter
        default: nil
        }
    }

    static func control(_ letter: Character) -> AgentKey? {
        controlLetters.first { $0.name == "ctrl+\(letter.lowercased())" }
    }

    init?(name: String) {
        guard let key = Self.all.first(where: { $0.name == name }) else { return nil }
        self = key
    }

    /// collied's name for the key.
    var name: String {
        switch self {
        case .esc: "esc"
        case .enter: "enter"
        case .up: "up"
        case .down: "down"
        case .left: "left"
        case .shiftLeft: "shift+left"
        case .right: "right"
        case .tab: "tab"
        case .shiftTab: "shift+tab"
        case .backspace: "backspace"
        case .ctrlC: "ctrl+c"
        case .ctrlEnter: "ctrl+enter"
        case .ctrlA: "ctrl+a"
        case .ctrlB: "ctrl+b"
        case .ctrlE: "ctrl+e"
        case .ctrlK: "ctrl+k"
        case .ctrlL: "ctrl+l"
        case .ctrlU: "ctrl+u"
        case .ctrlW: "ctrl+w"
        }
    }

    var symbol: String {
        switch self {
        case .esc: "E"
        case .enter: "⏎"
        case .up: "↑"
        case .down: "↓"
        case .left: "←"
        case .shiftLeft: "⇧←"
        case .right: "→"
        case .tab: "⇥"
        case .shiftTab: "⇧⇥"
        case .backspace: "⌫"
        case .ctrlEnter: "⌃⏎"
        case .ctrlC, .ctrlA, .ctrlB, .ctrlE, .ctrlK, .ctrlL, .ctrlU, .ctrlW: "^" + name.dropFirst(5).uppercased()
        }
    }

    var accessibilityName: String {
        switch self {
        case .esc: "Escape"
        case .enter: "Return"
        case .up: "Up arrow"
        case .down: "Down arrow"
        case .left: "Left arrow"
        case .shiftLeft: "Shift Left arrow"
        case .right: "Right arrow"
        case .tab: "Tab"
        case .shiftTab: "Shift Tab"
        case .backspace: "Backspace"
        case .ctrlEnter: "Control Return"
        case .ctrlC, .ctrlA, .ctrlB, .ctrlE, .ctrlK, .ctrlL, .ctrlU, .ctrlW: "Control " + name.dropFirst(5).uppercased()
        }
    }
}
