import CollieCore
import Foundation
import Observation

@MainActor
@Observable
final class NewTaskModel {
    enum Phase: Equatable {
        case editing
        case starting
        case waitingForAgent
        case startedNotVisible
    }

    private let core: any AgentCore
    private let prefsFile: URL?
    var machineId: String?
    private(set) var options: TaskOptions?
    private(set) var optionsError: String?
    private(set) var base: String?
    private(set) var folders: TaskFolders?
    private(set) var foldersError: String?
    var cwd = ""
    var newFolder = false
    var newFolderRoot: String?
    var folderName = ""
    var agent = ""
    var prompt = ""
    var label = ""
    private(set) var phase = Phase.editing
    private(set) var error: String?
    private(set) var cancelled = false

    init(core: any AgentCore, machines: [Machine], preferredMachineId: String? = nil, prefsFile: URL? = DevicePrefs.file) {
        self.core = core
        self.prefsFile = prefsFile
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
        guard phase == .editing, machineId != nil, !agent.isEmpty, !trimmed(prompt).isEmpty else { return false }
        return newFolder ? newFolderParent != nil && newFolderNameIsValid : folder != nil
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

    /// Starts the task, then waits for its agent to show in the flock so the agent screen
    /// can watch it. A started task is never offered again from this sheet. Returns nil once
    /// cancelled, even if the task started.
    func start() async -> AgentRoute? {
        guard canStart, !cancelled, let machineId, let cwd = newFolder ? newFolderParent : folder else { return nil }
        phase = .starting
        error = nil
        let started: TaskStarted
        do {
            started = try await core.taskNew(
                machineId: machineId, cwd: cwd, agent: agent, prompt: trimmed(prompt),
                label: trimmed(label).nilIfEmpty, newFolder: newFolder ? trimmed(folderName) : nil
            )
        } catch {
            self.error = AgentModel.message(for: error)
            // A folder made before the agent failed to start stays; start in it next time.
            let name = trimmed(folderName)
            if newFolder, let listed = try? await core.taskFolders(machineId: machineId, path: cwd),
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
    }

    private func trimmed(_ s: String) -> String {
        s.trimmingCharacters(in: .whitespacesAndNewlines)
    }
}
