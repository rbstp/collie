import CollieCore
import Foundation
import Observation

@MainActor
@Observable
final class NewTaskModel {
    enum Location: String, CaseIterable { case folder = "Folder", worktree = "Git worktree" }
    enum WorktreeAction: String, CaseIterable { case create = "Create", open = "Open" }
    enum Phase: Equatable {
        case editing
        case starting
        case waitingForAgent
        case startedNotVisible
    }

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
        }
    }
    private(set) var options: TaskOptions?
    private(set) var optionsError: String?
    private(set) var base: String?
    private(set) var folders: TaskFolders?
    private(set) var foldersError: String?
    var cwd = ""
    var location = Location.folder
    var worktreeAction = WorktreeAction.create
    var source = ""
    var branch = ""
    private let generatedBranch = "task-\(UUID().uuidString.lowercased())"
    var createBranch: String { trimmed(branch).nilIfEmpty ?? generatedBranch }
    var checkoutPath = ""
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
    var agent = ""
    var prompt = ""
    var label = ""
    private(set) var attachments: [AttachedFile] = []
    private(set) var upload: AttachmentUpload?
    private(set) var attachmentError: String?
    private var uploadTask: Task<Void, Never>?
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
    var folder: String? {
        let typed = trimmed(cwd)
        if typed.hasPrefix("/") { return typed }
        guard !typed.isEmpty, let base else { return nil }
        return base + "/" + typed
    }

    var newFolderParent: String? { base ?? newFolderRoot ?? options?.roots.first }

    var newFolderNameIsValid: Bool {
        let name = trimmed(folderName)
        return !name.isEmpty && !name.contains("/") && !name.hasPrefix(".")
    }

    var completions: [String] {
        let typed = trimmed(cwd)
        guard !typed.isEmpty, !typed.contains("/") else { return [] }
        return Array(
            (folders?.folders ?? []).filter { $0 != typed && $0.lowercased().hasPrefix(typed.lowercased()) }.prefix(5)
        )
    }

    var canStart: Bool {
        guard phase == .editing, machineId != nil, !agent.isEmpty, upload == nil, !dictation.isActive,
            !trimmed(prompt).isEmpty || !attachments.isEmpty else { return false }
        if location == .worktree {
            guard worktrees != nil, listedSource == trimmed(source) else { return false }
            if worktreeAction == .create {
                return createPath != nil
            }
            return worktrees?.worktrees.contains { $0.path == checkoutPath } == true
        }
        return newFolder ? newFolderParent != nil && newFolderNameIsValid : folder != nil
    }

    var attachmentSlots: Int { Attachment.maxPerPrompt - attachments.count }

    func remove(_ file: AttachedFile) {
        attachments.removeAll { $0.id == file.id }
    }

    @discardableResult
    func startDictation() -> Task<Void, Never>? {
        guard phase == .editing else { return nil }
        return dictation.start(appendingTo: prompt) { [weak self] in self?.prompt = $0 }
    }

    @discardableResult
    func attach(_ items: [PendingAttachment]) -> Task<Void, Never>? {
        guard phase == .editing, upload == nil, !items.isEmpty, let machineId else { return nil }
        attachmentError = nil
        let slots = max(attachmentSlots, 0)
        var firstError: (any Error)? = items.count > slots ? AttachmentError.tooMany(limit: Attachment.maxPerPrompt) : nil
        let batch = Array(items.prefix(slots))
        guard !batch.isEmpty else {
            attachmentError = firstError.map(AgentModel.message(for:))
            return nil
        }
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
            if !Task.isCancelled, let firstError { attachmentError = AgentModel.message(for: firstError) }
            if !Task.isCancelled { upload = nil }
        }
        uploadTask = task
        return task
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
        uploadTask?.cancel()
        uploadTask = nil
        upload = nil
    }

    private func uploaded(_ id: UUID, sent: UInt64, total: UInt64) {
        guard upload?.id == id, let current = upload, sent >= current.sent else { return }
        upload?.sent = sent
        upload?.total = total
    }

    func loadOptions() async {
        guard let machineId else { return }
        options = nil
        optionsError = nil
        newFolderRoot = nil
        let base = DevicePrefs.load(from: prefsFile).taskBases[machineId]
        self.base = base
        folders = nil
        foldersError = nil
        do {
            let loaded = try await core.taskOptions(machineId: machineId)
            guard machineId == self.machineId else { return }
            options = loaded
            if !loaded.agents.contains(agent) { agent = loaded.defaultAgent }
            if source.isEmpty { source = loaded.recentCwds.first ?? base ?? loaded.roots.first ?? "" }
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
        guard let machineId else { return }
        let requested = trimmed(source)
        worktrees = nil
        worktreesError = nil
        do {
            let listed = try await core.taskWorktrees(machineId: machineId, cwd: requested)
            guard machineId == self.machineId, requested == trimmed(source) else { return }
            listedSource = requested
            worktrees = listed
            if worktreeAction == .open && !listed.worktrees.contains(where: { $0.path == checkoutPath }) {
                checkoutPath = listed.worktrees.first?.path ?? ""
            }
        } catch {
            guard machineId == self.machineId else { return }
            worktreesError = AgentModel.message(for: error)
        }
    }

    /// Starts the task, then waits for its agent to show in the flock so the agent screen
    /// can watch it. A started task is never offered again from this sheet. Returns nil once
    /// cancelled, even if the task started.
    func start() async -> AgentRoute? {
        guard canStart, !cancelled, let machineId else { return nil }
        let text = (attachments.map(\.path) + [trimmed(prompt)].filter { !$0.isEmpty }).joined(separator: " ")
        phase = .starting
        error = nil
        let started: TaskStarted
        do {
            if location == .worktree {
                if worktreeAction == .create {
                    started = try await core.taskWorktreeCreate(
                        machineId: machineId, cwd: trimmed(source), branch: createBranch,
                        agent: agent, prompt: text, label: trimmed(label).nilIfEmpty
                    )
                } else {
                    started = try await core.taskWorktreeOpen(
                        machineId: machineId, cwd: trimmed(source), path: checkoutPath,
                        agent: agent, prompt: text, label: trimmed(label).nilIfEmpty
                    )
                }
            } else {
                guard let cwd = newFolder ? newFolderParent : folder else { phase = .editing; return nil }
                started = try await core.taskNew(
                    machineId: machineId, cwd: cwd, agent: agent, prompt: text,
                    label: trimmed(label).nilIfEmpty, newFolder: newFolder ? trimmed(folderName) : nil
                )
            }
        } catch {
            self.error = AgentModel.message(for: error)
            // A folder made before the agent failed to start stays; start in it next time.
            let name = trimmed(folderName)
            if location == .folder, newFolder, let cwd = newFolderParent,
                let listed = try? await core.taskFolders(machineId: machineId, path: cwd),
                listed.folders.contains(name)
            {
                newFolder = false
                self.cwd = listed.path + "/" + name
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
