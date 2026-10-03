import CollieCore
import SwiftUI

struct FlockScreen: View {
    let app: AppModel
    @State private var model = FlockModel()

    var body: some View {
        NavigationStack {
            List {
                if model.entries.isEmpty {
                    ContentUnavailableView(
                        "No machines yet",
                        systemImage: "desktopcomputer",
                        description: Text("Pair a Mac running collied from the Machines tab.")
                    )
                }
                ForEach(model.entries) { entry in
                    Section {
                        if let error = entry.error {
                            Label(error, systemImage: "exclamationmark.triangle")
                                .font(.footnote)
                                .foregroundStyle(.orange)
                        }
                        if entry.flock?.details != nil && entry.agents.isEmpty {
                            Text("No agents running").foregroundStyle(.secondary)
                        }
                        ForEach(entry.agents, id: \.terminalId) { agent in
                            AgentRow(agent: agent, workspace: entry.workspaceLabel(for: agent))
                        }
                    } header: {
                        MachineHeader(entry: entry)
                    }
                }
            }
            .navigationTitle("Flock 🐑")
            .refreshable { await model.refresh(core: app.core) }
            .task {
                while !Task.isCancelled {
                    await model.refresh(core: app.core)
                    try? await Task.sleep(for: .seconds(3))
                }
            }
        }
    }
}

private struct MachineHeader: View {
    let entry: MachineFlockEntry

    var body: some View {
        HStack {
            Text(entry.machine.label)
            Spacer()
            if let count = entry.flock?.approvalsCount, count > 0 {
                Text("\(count) approval\(count == 1 ? "" : "s")")
                    .foregroundStyle(.red)
            }
            if let link = entry.flock?.link {
                Text(link.label).foregroundStyle(link == .connected ? .green : .secondary)
            }
        }
    }
}

private struct AgentRow: View {
    let agent: AgentSummary
    let workspace: String?

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 12) {
            StatusPill(state: agent.status)
            VStack(alignment: .leading, spacing: 2) {
                Text(agent.displayTitle).lineLimit(2)
                if let subtitle = [workspace, agent.kind].compactMap({ $0 }).joined(separator: " · ").nilIfEmpty {
                    Text(subtitle).font(.caption).foregroundStyle(.secondary)
                }
            }
            Spacer()
            TimelineView(.periodic(from: .now, by: 1)) { context in
                Text(Elapsed.string(sinceMs: agent.statusSinceMs, now: context.date))
                    .font(.caption.monospacedDigit())
                    .foregroundStyle(.secondary)
            }
        }
    }
}

struct StatusPill: View {
    let state: AgentState

    var body: some View {
        Text(state.label)
            .font(.caption.bold())
            .padding(.horizontal, 8)
            .padding(.vertical, 3)
            .background(state.color.opacity(0.18), in: Capsule())
            .foregroundStyle(state.color)
    }
}

extension AgentState {
    var label: String {
        switch self {
        case .idle: "idle"
        case .working: "working"
        case .blocked: "blocked"
        case .done: "done"
        case .unknown: "unknown"
        }
    }

    var color: Color {
        switch self {
        case .blocked: .red
        case .working: .blue
        case .idle: .gray
        case .done: .green
        case .unknown: .secondary
        }
    }
}

extension LinkPhase {
    var label: String {
        switch self {
        case .connecting: "connecting"
        case .connected: "connected"
        case .waiting: "retrying"
        case .offline: "offline"
        case .stopped: "stopped"
        }
    }
}

extension String {
    var nilIfEmpty: String? { isEmpty ? nil : self }
}
