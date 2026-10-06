import CollieCore
import SwiftUI

struct FlockScreen: View {
    let core: (any FlockCore)?
    let machines: [Machine]
    let approvals: ApprovalsModel?
    let follows: FollowModel?
    @Binding var opening: AgentRoute?
    var tailnetStarting = false
    @State private var model = FlockModel()
    @State private var path: [AgentRoute] = []
    @State private var newTask = false
    @State private var previews = PreviewModel()
    @State private var layout = DevicePrefs.load(from: DevicePrefs.file).agentsLayout
    @Environment(\.scenePhase) private var scenePhase

    var body: some View {
        NavigationStack(path: $path) {
            Group {
                switch layout {
                case .grid:
                    AgentGrid(
                        entries: model.entries, previews: previews, notice: model.closeNotice,
                        approvalsCount: approvalsCount, reconnect: reconnect, showsLink: !tailnetStarting, follows: follows, menu: menu
                    )
                    .task(id: core != nil && scenePhase == .active && !newTask) {
                        guard let core, scenePhase == .active, !newTask else { return }
                        await previews.run(core: core)
                    }
                case .inbox:
                    AgentInbox(
                        entries: model.entries, notice: model.closeNotice, showsMachine: machines.count > 1,
                        reconnect: reconnect, follows: follows, menu: menu
                    )
                case .list:
                    List {
                        if let notice = model.closeNotice {
                            Label(notice, systemImage: "exclamationmark.triangle")
                                .font(.footnote)
                                .foregroundStyle(.orange)
                        }
                        if model.entries.isEmpty {
                            NoMachines()
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
                                        AgentRow(
                                            agent: agent, workspace: entry.workspaceLabel(for: agent),
                                            followed: follows?.isFollowing(route) == true
                                        )
                                    }
                                    .contextMenu { menu(agent, route) }
                                }
                                .opacity(entry.linkDown ? 0.5 : 1)
                            } header: {
                                Button {
                                    reconnect(entry)
                                } label: {
                                    MachineHeader(entry: entry, approvalsCount: approvalsCount(entry), showsLink: !tailnetStarting)
                                }
                                .buttonStyle(.plain)
                            }
                        }
                    }
                }
            }
            .navigationTitle("Agents")
            .navigationBarTitleDisplayMode(.inline)
            .refreshable {
                for machine in machines { try? core?.reconnect(machineId: machine.id) }
                await model.refresh(core: core)
            }
            .toolbar {
                if tailnetStarting {
                    ToolbarItem(placement: .principal) {
                        HStack(spacing: 6) {
                            ProgressView().controlSize(.small)
                            Text("Connecting…").font(.headline)
                        }
                    }
                }
                ToolbarItem(placement: .primaryAction) {
                    Menu("Layout", systemImage: layout.icon) {
                        Picker("Layout", selection: $layout) {
                            ForEach(AgentsLayout.allCases, id: \.self) { Label($0.label, systemImage: $0.icon) }
                        }
                    }
                }
                ToolbarItem(placement: .primaryAction) {
                    Button("New task", systemImage: "plus") { newTask = true }
                        .disabled(core == nil || machines.isEmpty)
                }
            }
            .closeConfirmation($model.close) {
                if let core, await model.performClose(core: core) {
                    await model.refresh(core: core)
                }
            }
            .navigationDestination(for: AgentRoute.self) { route in
                if let core {
                    AgentScreen(
                        core: core, route: route, approvals: approvals, follows: follows,
                        machineLabel: machines.first { $0.id == route.machineId }?.label, showsMachine: machines.count > 1,
                        neighbor: { offset in
                            // model.entries stops refreshing under a pushed agent; the core's cache follows events.
                            let entries = core.machines().map { MachineFlockEntry(machine: $0, flock: core.cachedFlock(machineId: $0.id)) }
                            return FlockOrder.neighbor(of: route, offset: offset, in: entries)
                        },
                        switchAgent: { next in
                            // Replaces the top route, so Back still returns to this list.
                            if path.last == route { path[path.count - 1] = next }
                        }
                    )
                    // A replaced route must get its own model, not keep the previous agent's.
                    .id(route)
                }
            }
            .sheet(isPresented: $newTask) {
                if let core {
                    NewTaskSheet(
                        core: core, machines: machines,
                        preferredMachineId: model.entries.first { $0.flock?.link == .connected }?.id
                    ) { path.append($0) }
                }
            }
            .task {
                while !Task.isCancelled {
                    await model.refresh(core: core)
                    follows?.sync()
                    try? await Task.sleep(for: .seconds(3))
                }
            }
            .onChange(of: model.entries, initial: true) { _, entries in previews.update(entries) }
            .onChange(of: layout) { _, layout in
                var prefs = DevicePrefs.load(from: DevicePrefs.file)
                prefs.agentsLayout = layout
                prefs.save(to: DevicePrefs.file)
            }
            .onChange(of: opening, initial: true) { _, route in
                guard let route else { return }
                path = [route]
                opening = nil
            }
        }
        .followNotice(follows)
    }

    private func approvalsCount(_ entry: MachineFlockEntry) -> Int {
        approvals.map { $0.items.filter { $0.machine.id == entry.id }.count } ?? Int(entry.flock?.approvalsCount ?? 0)
    }

    private func reconnect(_ entry: MachineFlockEntry) {
        try? core?.reconnect(machineId: entry.id)
    }

    @ViewBuilder
    private func menu(_ agent: AgentSummary, _ route: AgentRoute) -> some View {
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

struct NoMachines: View {
    var body: some View {
        ContentUnavailableView(
            "No machines yet",
            systemImage: "desktopcomputer",
            description: Text("Pair a machine running collied from the Machines tab.")
        )
    }
}

struct MachineHeader: View {
    let entry: MachineFlockEntry
    let approvalsCount: Int
    let showsLink: Bool

    var body: some View {
        HStack {
            MachineName(machine: entry.machine)
            Spacer()
            if approvalsCount > 0 {
                Text("\(approvalsCount) approval\(approvalsCount == 1 ? "" : "s")")
                    .foregroundStyle(.red)
            }
            if showsLink, let link = entry.flock?.link {
                if link == .connecting {
                    ProgressView().controlSize(.mini)
                    Text(entry.flock?.details != nil ? "reconnecting" : "connecting").foregroundStyle(.secondary)
                } else {
                    Text(link.label).foregroundStyle(link == .connected ? .green : .secondary)
                }
            }
        }
    }
}

private struct AgentRow: View {
    let agent: AgentSummary
    let workspace: String?
    let followed: Bool

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 12) {
            StatusIcon(state: agent.status)
                // Centered on the title's first line rather than sitting on its baseline.
                .alignmentGuide(.firstTextBaseline) { $0[VerticalAlignment.center] + 6 }
            VStack(alignment: .leading, spacing: 2) {
                HStack(alignment: .firstTextBaseline, spacing: 4) {
                    if followed {
                        Image(systemName: "pin.fill")
                            .font(.caption)
                            .foregroundStyle(.tint)
                            .accessibilityLabel("Followed")
                    }
                    Text(agent.displayTitle).lineLimit(2)
                }
                if workspace != nil || agent.kind != nil {
                    HStack(spacing: 4) {
                        if let workspace { Text(workspace) }
                        if let kind = agent.kind {
                            if workspace != nil { Text(verbatim: "·") }
                            AgentKindLabel(kind: kind).fixedSize()
                        }
                    }
                    .lineLimit(1)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                }
            }
            Spacer()
            TimelineView(.periodic(from: .now, by: 1)) { context in
                Text(Elapsed.string(sinceMs: agent.statusSinceMs, now: context.date))
                    .font(.caption.monospacedDigit())
                    .foregroundStyle(.secondary)
            }
            // A fixed slot, so times line up whether or not a row has a ring.
            Group {
                if let left = agent.contextLeft { ContextRing(left: left) }
            }
            .frame(width: ContextRing.size)
        }
    }
}

/// The agent kind as herdr names it, with its own icon for the kinds New Task starts.
struct AgentKindLabel: View {
    let kind: String
    var iconOnly = false

    var body: some View {
        let label = Group {
            switch kind {
            case "claude": Label("Claude", image: "Claude")
            case "codex": Label("Codex", image: "Codex")
            case "copilot": Label("Copilot", image: "Copilot")
            default: Label(kind, systemImage: "terminal")
            }
        }
        if iconOnly {
            label.labelStyle(.iconOnly)
        } else {
            label.labelStyle(KindLabelStyle())
        }
    }
}

private struct KindLabelStyle: LabelStyle {
    func makeBody(configuration: Configuration) -> some View {
        HStack(spacing: 4) {
            configuration.icon
            configuration.title
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

struct StatusIcon: View {
    let state: AgentState

    var body: some View {
        Group {
            switch state {
            case .working: SpinningRing(color: state.color)
            case .done: Image(systemName: "checkmark.circle.fill").foregroundStyle(state.color)
            case .idle: Image(systemName: "circle.fill").font(.system(size: 9)).foregroundStyle(state.color)
            case .blocked: Image(systemName: "exclamationmark.circle.fill").foregroundStyle(state.color)
            case .unknown: Image(systemName: "questionmark.circle").foregroundStyle(state.color)
            }
        }
        .font(.system(size: 18))
        .frame(width: 20, height: 20)
        .accessibilityElement()
        .accessibilityLabel(state.label)
    }
}

/// Context left in the agent's window: a static arc, unlike the working spinner.
struct ContextRing: View {
    static let size: CGFloat = 14
    let left: UInt8

    var body: some View {
        let tone = Self.tone(left)
        ZStack {
            Circle().stroke(tone.opacity(0.25), lineWidth: 2)
            Circle()
                .trim(from: 0, to: Double(min(left, 100)) / 100)
                .stroke(tone, style: StrokeStyle(lineWidth: 2, lineCap: .round))
                .rotationEffect(.degrees(-90))
        }
        .padding(1)
        .frame(width: Self.size, height: Self.size)
        .accessibilityElement()
        .accessibilityLabel("Context \(left)% left")
    }

    static func tone(_ left: UInt8) -> Color {
        switch left {
        case 51...: Color.teal.mix(with: .gray, by: 0.45)
        case 20...: Color.orange.mix(with: .yellow, by: 0.25)
        default: Color.pink.mix(with: .orange, by: 0.45)
        }
    }
}

/// Its own view so every switch to working starts a fresh spin.
private struct SpinningRing: View {
    let color: Color
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var spinning = false

    var body: some View {
        Circle()
            .trim(from: 0, to: 0.7)
            .stroke(color, style: StrokeStyle(lineWidth: 2.5, lineCap: .round))
            .padding(2)
            .rotationEffect(.degrees(spinning ? 360 : 0))
            .animation(reduceMotion ? nil : .linear(duration: 1).repeatForever(autoreverses: false), value: spinning)
            .onAppear { spinning = true }
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
        case .unavailable: "offline"
        case .offline: "offline"
        case .stopped: "stopped"
        }
    }
}

extension String {
    var nilIfEmpty: String? { isEmpty ? nil : self }
}
