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
    var machineId: String?
    private(set) var options: TaskOptions?
    private(set) var optionsError: String?
    var cwd = ""
    var agent = ""
    var prompt = ""
    var label = ""
    private(set) var phase = Phase.editing
    private(set) var error: String?

    init(core: any AgentCore, machines: [Machine]) {
        self.core = core
        machineId = machines.first?.id
    }

    var canStart: Bool {
        phase == .editing && machineId != nil && !agent.isEmpty
            && trimmed(cwd).hasPrefix("/") && !trimmed(prompt).isEmpty
    }

    func loadOptions() async {
        guard let machineId else { return }
        options = nil
        optionsError = nil
        do {
            let loaded = try await core.taskOptions(machineId: machineId)
            guard machineId == self.machineId else { return }
            options = loaded
            if !loaded.agents.contains(agent) { agent = loaded.defaultAgent }
        } catch {
            optionsError = AgentModel.message(for: error)
        }
    }

    /// Starts the task, then waits for its agent to show in the flock so the agent screen
    /// can watch it. A started task is never offered again from this sheet.
    func start() async -> AgentRoute? {
        guard canStart, let machineId else { return nil }
        phase = .starting
        error = nil
        let started: TaskStarted
        do {
            started = try await core.taskNew(
                machineId: machineId, cwd: trimmed(cwd), agent: agent, prompt: trimmed(prompt),
                label: trimmed(label).nilIfEmpty
            )
        } catch {
            phase = .editing
            self.error = AgentModel.message(for: error)
            return nil
        }
        phase = .waitingForAgent
        let route = AgentRoute(machineId: machineId, terminalId: started.terminalId)
        for _ in 0..<30 where !Task.isCancelled {
            if let flock = try? await core.flock(machineId: machineId),
                flock.agents.contains(where: { $0.terminalId == route.terminalId })
            {
                return route
            }
            try? await Task.sleep(for: .seconds(1))
        }
        phase = .startedNotVisible
        error = "The task started, but its agent has not appeared yet. It will show in the flock."
        return nil
    }

    private func trimmed(_ s: String) -> String {
        s.trimmingCharacters(in: .whitespacesAndNewlines)
    }
}
