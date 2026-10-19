import CollieCore
import Foundation
import Observation
import SwiftUI

extension TaskFolders {
    func folderPath(_ name: String) -> String {
        URL(filePath: path).appending(path: name).path
    }
}

@MainActor
@Observable
final class NewTaskModel {
    enum Location: String, CaseIterable { case folder = "Folder", worktree = "Git worktree", clone = "Clone", resume = "Resume" }
    enum WorktreeAction: String, CaseIterable { case create = "Create", open = "Open", pullRequest = "Pull request" }
    enum Phase: Equatable {
        case editing
        case starting
        case waitingForAgent
        case startedNotVisible
    }

    /// collied's `drive::CODEX_UPDATE`, at the end of its `agent_blocked` message.
    static let codexUpdate = "Codex is asking to update. Answer it in Terminal."

    private let core: any AgentCore
    private let prefsFile: URL?
    var machineId: String? {
        didSet {
            guard oldValue != machineId else { return }
            cancelUpload(on: oldValue)
            attachments = []
            attachmentError = nil
            worktrees = nil
            worktreesError = nil
            source = ""
            checkoutPath = ""
            sessions = nil
            sessionsError = nil
            session = nil
        }
    }
    private(set) var options: TaskOptions?
    private(set) var optionsError: String?
    private(set) var base: String?
    private(set) var folders: TaskFolders?
    private(set) var foldersError: String?
    var cwd = "" {
        didSet {
            guard oldValue != cwd else { return }
            sessions = nil
            sessionsError = nil
            session = nil
        }
    }
    private(set) var sessions: [TaskSession]?
    private(set) var sessionsError: String?
    private var sessionsFolder = ""
    var session: TaskSession?
    var location = Location.folder {
        didSet {
            guard oldValue != location, location == .worktree, !newFolder, let folder else { return }
            source = folder
        }
    }
    var worktreeAction = WorktreeAction.create
    var source = "" {
        didSet {
            guard oldValue != source else { return }
            worktrees = nil
            worktreesError = nil
        }
    }
    var branch = ""
    private let generatedBranch = "task-\(UUID().uuidString.lowercased())"
    var createBranch: String { trimmed(branch).nilIfEmpty ?? generatedBranch }
    var checkoutPath = ""
    var pullRequestNumber = ""
    var pullRequest: UInt64? {
        let typed = trimmed(pullRequestNumber)
        guard typed.allSatisfy({ ("0"..."9").contains($0) }), let number = UInt64(typed), number > 0, number <= UInt32.max else { return nil }
        return number
    }
    private(set) var worktrees: WorktreeListing?
    private(set) var worktreesError: String?
    private var listedSource = ""
    var createPath: String? {
        guard let worktrees else { return nil }
        return worktrees.source + "/.worktree/" + createBranch.replacingOccurrences(of: "/", with: "-")
    }
    var newFolder = false
    var newFolderRoot: String?
    var folderName = ""
    var repository = ""
    var cloneFolder: String? { cloneFolderName(repository: trimmed(repository)) }
    private(set) var cloning: String?
    var agent = ""
    var ultracode = false {
        didSet {
            guard oldValue != ultracode, let machineId else { return }
            DevicePrefs.update(in: prefsFile) { $0.taskUltracode[machineId] = ultracode }
        }
    }
    var prompt = ""
    var label = ""
    private(set) var attachments: [AttachedFile] = []
    private let uploads = AttachmentUploads()
    var upload: AttachmentUpload? { uploads.current }
    private(set) var attachmentError: String?
    let dictation: DictationModel
    private(set) var phase = Phase.editing
    private(set) var error: String?
    private(set) var cancelled = false

    init(
        core: any AgentCore, machines: [Machine], preferredMachineId: String? = nil,
        prefsFile: URL? = DevicePrefs.file, dictationEngine: any DictationEngine = SpeechDictationEngine()
    ) {
        self.core = core
        self.prefsFile = prefsFile
        dictation = DictationModel(
            engine: dictationEngine, language: DevicePrefs.load(from: prefsFile).dictationLanguage, prefsFile: prefsFile
        )
        machineId = machines.first { $0.id == preferredMachineId }?.id ?? machines.first?.id
    }

    /// collied resolves this and checks it against its roots.
    var folder: String? { resolvedFolder(cwd) }
    var sourceFolder: String? { resolvedFolder(source) }

    private func resolvedFolder(_ path: String) -> String? {
        let typed = trimmed(path)
        if typed.hasPrefix("/") { return typed }
        guard !typed.isEmpty, let base else { return nil }
        return base + "/" + typed
    }

    var newFolderParent: String? { base ?? newFolderRoot ?? options?.roots.first }

    var newFolderNameIsValid: Bool {
        let name = trimmed(folderName)
        return !name.isEmpty && !name.contains("/") && !name.hasPrefix(".")
    }

    var completions: [String] { folderCompletions(cwd) }
    var sourceCompletions: [String] { folderCompletions(source) }

    private func folderCompletions(_ path: String) -> [String] {
        let typed = trimmed(path)
        guard !typed.isEmpty, !typed.contains("/") else { return [] }
        return Array(
            (folders?.folders ?? []).filter { $0 != typed && $0.lowercased().hasPrefix(typed.lowercased()) }.prefix(5)
        )
    }

    var canStart: Bool {
        guard !cancelled, phase == .editing, machineId != nil, upload == nil, !dictation.isActive else { return false }
        if location == .resume {
            guard let session, let folder, sessionsFolder == folder else { return false }
            return sessions?.contains { $0.sessionId == session.sessionId } == true
        }
        guard !agent.isEmpty, !trimmed(prompt).isEmpty || !attachments.isEmpty else { return false }
        if location == .worktree {
            guard let sourceFolder, worktrees != nil, listedSource == sourceFolder else { return false }
            switch worktreeAction {
            case .create: return createPath != nil
            case .open: return worktrees?.worktrees.contains { $0.path == checkoutPath } == true
            case .pullRequest: return pullRequest != nil
            }
        }
        if location == .clone {
            return newFolderParent != nil && cloneFolder != nil
        }
        return newFolder ? newFolderParent != nil && newFolderNameIsValid : folder != nil
    }

    var attachmentSlots: Int { Attachment.maxPerPrompt - attachments.count }

    func remove(_ file: AttachedFile) {
        attachments.removeAll { $0.id == file.id }
    }

    @discardableResult
    func startDictation(at selection: TextSelection? = nil) -> Task<Void, Never>? {
        guard phase == .editing else { return nil }
        return dictation.start(in: prompt, at: selection) { [weak self] in self?.prompt = $0 }
    }

    @discardableResult
    func attach(_ items: [PendingAttachment]) -> Task<Void, Never>? {
        guard phase == .editing, upload == nil, !items.isEmpty, let machineId else { return nil }
        attachmentError = nil
        return uploads.start(items, slots: attachmentSlots, core: core, machineId: machineId) { file in
            self.attachments.append(file)
        } failed: { message in
            self.attachmentError = message
        }
    }

    func attachFailed(_ error: any Error) {
        attachmentError = AgentModel.message(for: error)
    }

    func cancelUpload() {
        cancelUpload(on: machineId)
    }

    private func cancelUpload(on machineId: String?) {
        guard upload != nil else { return }
        if let machineId { core.cancelUploads(machineId: machineId) }
        uploads.cancel()
    }

    func loadOptions() async {
        guard let machineId else { return }
        options = nil
        optionsError = nil
        newFolderRoot = nil
        let prefs = DevicePrefs.load(from: prefsFile)
        let base = prefs.taskBases[machineId]
        self.base = base
        ultracode = prefs.taskUltracode[machineId] ?? false
        folders = nil
        foldersError = nil
        do {
            let loaded = try await core.taskOptions(machineId: machineId)
            guard machineId == self.machineId else { return }
            options = loaded
            if !loaded.agents.contains(agent) { agent = loaded.defaultAgent }
        } catch {
            guard machineId == self.machineId else { return }
            optionsError = AgentModel.message(for: error)
            return
        }
        guard let base else { return }
        do {
            let listed = try await core.taskFolders(machineId: machineId, path: base)
            guard machineId == self.machineId else { return }
            folders = listed
        } catch {
            guard machineId == self.machineId else { return }
            foldersError = "Base folder: \(AgentModel.message(for: error))"
        }
    }

    func loadWorktrees() async {
        guard let machineId, let requested = sourceFolder else { return }
        worktrees = nil
        worktreesError = nil
        do {
            let listed = try await core.taskWorktrees(machineId: machineId, cwd: requested)
            guard machineId == self.machineId, requested == sourceFolder else { return }
            listedSource = requested
            worktrees = listed
            if worktreeAction == .open && !listed.worktrees.contains(where: { $0.path == checkoutPath }) {
                checkoutPath = listed.worktrees.first?.path ?? ""
            }
        } catch {
            guard machineId == self.machineId, requested == sourceFolder else { return }
            worktreesError = AgentModel.message(for: error)
        }
    }

    func loadSessions() async {
        guard let machineId, let requested = folder else { return }
        sessions = nil
        sessionsError = nil
        do {
            let listed = try await core.taskSessions(machineId: machineId, cwd: requested)
            guard machineId == self.machineId, requested == folder else { return }
            sessionsFolder = requested
            sessions = listed
            if !listed.contains(where: { $0.sessionId == session?.sessionId }) { session = nil }
        } catch {
            guard machineId == self.machineId, requested == folder else { return }
            sessionsError = AgentModel.message(for: error)
        }
    }

    /// Starts the task, then waits for its agent to show in the flock so the agent screen
    /// can watch it. A started task is never offered again from this sheet. Returns nil once
    /// cancelled, even if the task started.
    func start() async -> AgentRoute? {
        guard canStart, !cancelled, let machineId else { return nil }
        let text = (attachments.map(\.path) + [trimmed(prompt)].filter { !$0.isEmpty }).joined(separator: " ")
        let ultracode = agent == "claude" && self.ultracode
        phase = .starting
        error = nil
        let started: TaskStarted
        do {
            if location == .worktree {
                guard let sourceFolder else { phase = .editing; return nil }
                switch worktreeAction {
                case .create:
                    started = try await core.taskWorktreeCreate(
                        machineId: machineId, cwd: sourceFolder, branch: createBranch,
                        agent: agent, prompt: text, label: trimmed(label).nilIfEmpty, ultracode: ultracode
                    )
                case .open:
                    started = try await core.taskWorktreeOpen(
                        machineId: machineId, cwd: sourceFolder, path: checkoutPath,
                        agent: agent, prompt: text, label: trimmed(label).nilIfEmpty, ultracode: ultracode
                    )
                case .pullRequest:
                    guard let number = pullRequest else { phase = .editing; return nil }
                    started = try await core.taskWorktreePullRequest(
                        machineId: machineId, cwd: sourceFolder, number: number,
                        agent: agent, prompt: text, label: trimmed(label).nilIfEmpty, ultracode: ultracode
                    )
                }
            } else if location == .resume {
                guard let cwd = folder, let session else { phase = .editing; return nil }
                started = try await core.taskResume(
                    machineId: machineId, cwd: cwd, agent: session.agent, sessionId: session.sessionId,
                    prompt: text, label: trimmed(label).nilIfEmpty
                )
            } else if location == .clone {
                guard let cwd = newFolderParent else { phase = .editing; return nil }
                cloning = trimmed(repository)
                defer { cloning = nil }
                started = try await core.taskClone(
                    machineId: machineId, cwd: cwd, repository: trimmed(repository),
                    agent: agent, prompt: text, label: trimmed(label).nilIfEmpty, ultracode: ultracode
                )
            } else {
                guard let cwd = newFolder ? newFolderParent : folder else { phase = .editing; return nil }
                started = try await core.taskNew(
                    machineId: machineId, cwd: cwd, agent: agent, prompt: text,
                    label: trimmed(label).nilIfEmpty, newFolder: newFolder ? trimmed(folderName) : nil,
                    ultracode: ultracode
                )
            }
        } catch {
            if case .AgentBlocked(let message) = error as? CoreError, message.hasSuffix(Self.codexUpdate) {
                self.error = Self.codexUpdate
            } else {
                self.error = AgentModel.message(for: error)
            }
            // A folder made or cloned before the agent failed to start stays; start in it next time.
            let made = location == .clone ? cloneFolder : location == .folder && newFolder ? trimmed(folderName) : nil
            if let name = made, let cwd = newFolderParent,
                let listed = try? await core.taskFolders(machineId: machineId, path: cwd),
                listed.folders.contains(name)
            {
                let path = listed.path + "/" + name
                if location == .clone {
                    // Only a failure after the clone completed: anything else may name an
                    // unrelated folder or a clone still running.
                    switch error as? CoreError {
                    case .AgentBlocked, .AgentNotReady, .HerdrUnavailable, .NotFound:
                        self.error = (self.error ?? "") + " The clone stays in \(path)."
                    default:
                        guard self.error?.contains("folder \(path) was cloned and left in place") == true else {
                            phase = .editing
                            return nil
                        }
                    }
                }
                location = .folder
                newFolder = false
                self.cwd = path
            }
            phase = .editing
            return nil
        }
        phase = .waitingForAgent
        let route = AgentRoute(machineId: machineId, terminalId: started.terminalId)
        for _ in 0..<30 where !cancelled {
            if let flock = try? await core.flock(machineId: machineId),
                flock.agents.contains(where: { $0.terminalId == route.terminalId })
            {
                return cancelled ? nil : route
            }
            try? await Task.sleep(for: .seconds(1))
        }
        phase = .startedNotVisible
        error = "The task started, but its agent has not appeared yet. It will show in the flock."
        return nil
    }

    func cancel() {
        cancelled = true
        dictation.cancel()
        cancelUpload()
    }

    private func trimmed(_ s: String) -> String {
        s.trimmingCharacters(in: .whitespacesAndNewlines)
    }
}
