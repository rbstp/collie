import CollieCore
import Foundation

struct ScheduledDrafts: StateFile {
    struct Pending: Codable {
        var startAtMs: UInt64
        var draft: AgentDrafts.Draft
    }

    static let file: URL? = try? StateDirectory.prepare().appending(path: "scheduled-drafts.json")
    static let alertThread = "collie-scheduled"
    @MainActor private static var settling: Set<String> = []
    @MainActor private(set) static var restored: Set<AgentRoute> = []

    var pending: [String: [String: Pending]] = [:]

    static func keep(_ draft: AgentDrafts.Draft, for task: ScheduledTask, machineId: String, file: URL?) {
        var saved = load(from: file)
        saved.pending[machineId, default: [:]][task.scheduleId] = Pending(startAtMs: task.startAtMs, draft: draft)
        saved.save(to: file)
    }

    static func kept(machineId: String, file: URL?) -> [String: Pending] {
        load(from: file).pending[machineId] ?? [:]
    }

    static func forget(machineId: String, file: URL?) {
        var saved = load(from: file)
        guard saved.pending.removeValue(forKey: machineId) != nil else { return }
        saved.save(to: file)
    }

    static func forget(_ scheduleId: String, machineId: String, file: URL?) {
        var saved = load(from: file)
        guard saved.pending[machineId]?.removeValue(forKey: scheduleId) != nil else { return }
        saved.save(to: file)
    }

    @MainActor
    static func reconcile(machineId: String, core: any AgentCore, now: Date = .now, file: URL?, draftsFile: URL?) async {
        let mine = kept(machineId: machineId, file: file)
        guard mine.values.contains(where: { Double($0.startAtMs) / 1000 <= now.timeIntervalSince1970 }), !settling.contains(machineId),
            let tasks = try? await core.taskScheduled(machineId: machineId)
        else { return }
        await settle(tasks, kept: mine, machineId: machineId, core: core, file: file, draftsFile: draftsFile)
    }

    @MainActor
    static func settle(
        _ tasks: [ScheduledTask], kept: [String: Pending], machineId: String, core: any AgentCore, file: URL?, draftsFile: URL?
    ) async {
        guard !kept.isEmpty, settling.insert(machineId).inserted else { return }
        defer { settling.remove(machineId) }
        var settled: Set<String> = []
        for (scheduleId, pending) in kept {
            guard let task = tasks.first(where: { $0.scheduleId == scheduleId }) else {
                settled.insert(scheduleId)
                continue
            }
            guard let terminalId = task.blocked else { continue }
            let route = AgentRoute(machineId: machineId, terminalId: terminalId)
            var drafts = AgentDrafts.load(from: draftsFile)
            drafts.drafts[route] = pending.draft
            drafts.save(to: draftsFile)
            restored.insert(route)
            settled.insert(scheduleId)
            try? await core.taskUnschedule(machineId: machineId, scheduleId: scheduleId)
        }
        var saved = load(from: file)
        saved.pending[machineId] = saved.pending[machineId]?.filter { !settled.contains($0.key) }
        if saved.pending[machineId]?.isEmpty == true { saved.pending[machineId] = nil }
        saved.save(to: file)
    }

    @MainActor
    static func seen(_ entries: [MachineFlockEntry]) {
        restored = restored.filter { route in
            guard let flock = entries.first(where: { $0.id == route.machineId })?.flock else { return true }
            return !flock.agents.contains { $0.terminalId == route.terminalId }
        }
    }
}
