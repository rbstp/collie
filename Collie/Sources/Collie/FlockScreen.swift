import CollieCore
import SwiftUI

struct FlockScreen: View {
    let core: (any FlockCore)?
    let machines: [Machine]
    let approvals: ApprovalsModel?
    let follows: FollowModel?
    @Binding var opening: AgentRoute?
    @State private var model = FlockModel()
    @State private var path: [AgentRoute] = []
    @State private var newTask = false

    var body: some View {
        NavigationStack(path: $path) {
            List {
                if let notice = model.closeNotice {
                    Label(notice, systemImage: "exclamationmark.triangle")
                        .font(.footnote)
                        .foregroundStyle(.orange)
                }
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
                            let route = AgentRoute(machineId: entry.id, terminalId: agent.terminalId)
                            NavigationLink(value: route) {
                                AgentRow(agent: agent, workspace: entry.workspaceLabel(for: agent))
                            }
                            .contextMenu {
                                if let follows {
                                    FollowMenuItem(follows: follows, route: route)
                                    Divider()
                                }
                                Button("Close pane", systemImage: "xmark.square", role: .destructive) {
                                    model.beginClose(.pane, route: route)
                                }
                                Button("Close workspace", systemImage: "xmark.rectangle.portrait", role: .destructive) {
                                    model.beginClose(.workspace(id: agent.workspaceId), route: route)
                                }
                            }
                        }
                    } header: {
                        MachineHeader(entry: entry)
                    }
                }
            }
            .navigationTitle("Agents")
            .navigationBarTitleDisplayMode(.inline)
            .refreshable { await model.refresh(core: core) }
            .toolbar {
                Button("New task", systemImage: "plus") { newTask = true }
                    .disabled(core == nil || machines.isEmpty)
            }
            .closeConfirmation($model.close) {
                if let core, await model.performClose(core: core) {
                    await model.refresh(core: core)
                }
            }
            .navigationDestination(for: AgentRoute.self) { route in
                if let core {
                    AgentScreen(core: core, route: route, approvals: approvals, follows: follows)
                }
            }
            .sheet(isPresented: $newTask) {
                if let core {
                    NewTaskSheet(core: core, machines: machines) { path.append($0) }
                }
            }
            .task {
                while !Task.isCancelled {
                    await model.refresh(core: core)
                    follows?.sync()
                    try? await Task.sleep(for: .seconds(3))
                }
            }
            .onChange(of: opening, initial: true) { _, route in
                guard let route else { return }
                path = [route]
                opening = nil
            }
        }
        .followNotice(follows)
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
        ZStack {
            // The widest labels size every pill, so titles line up across rows.
            Text("working").hidden()
            Text("unknown").hidden()
            Text(state.label)
        }
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
