import CollieCore
import GhosttyTerminal
import SwiftUI

struct AgentGrid<Menu: View>: View {
    let entries: [MachineFlockEntry]
    let previews: PreviewModel
    let notice: String?
    let approvalsCount: (MachineFlockEntry) -> Int
    let reconnect: (MachineFlockEntry) -> Void
    let showsLink: Bool
    let follows: FollowModel?
    let closePane: (AgentRoute) -> Void
    @ViewBuilder let menu: (AgentSummary, AgentRoute) -> Menu
    @Environment(\.dynamicTypeSize) private var typeSize

    var body: some View {
        TimelineView(.periodic(from: .now, by: 60)) { context in
            ScrollView {
                if let notice {
                    Label(notice, systemImage: "exclamationmark.triangle")
                        .font(.footnote)
                        .foregroundStyle(.orange)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .padding(.horizontal)
                }
                if entries.isEmpty {
                    NoMachines()
                }
                LazyVStack(alignment: .leading, spacing: 16) {
                    ForEach(entries) { entry in
                        Section {
                            let agents = entry.gridAgents(starred: previews.starred)
                            ForEach(agents.starred, id: \.terminalId) { card($0, in: entry, starred: true, now: context.date) }
                            if !agents.rest.isEmpty {
                                LazyVGrid(
                                    columns: typeSize.isAccessibilitySize ? [GridItem(.flexible())] : [GridItem(.adaptive(minimum: 150), spacing: 12)],
                                    alignment: .leading, spacing: 16
                                ) {
                                    ForEach(agents.rest, id: \.terminalId) { card($0, in: entry, starred: false, now: context.date) }
                                }
                            }
                            if !entry.terminals.isEmpty {
                                Text("Terminals").font(.caption).foregroundStyle(.secondary)
                                LazyVGrid(
                                    columns: typeSize.isAccessibilitySize ? [GridItem(.flexible())] : [GridItem(.adaptive(minimum: 150), spacing: 12)],
                                    alignment: .leading, spacing: 12
                                ) {
                                    ForEach(entry.terminals, id: \.terminalId) { terminalCard($0, in: entry) }
                                }
                            }
                        } header: {
                            VStack(alignment: .leading, spacing: 6) {
                                Button {
                                    reconnect(entry)
                                } label: {
                                    MachineHeader(entry: entry, approvalsCount: approvalsCount(entry), showsLink: showsLink)
                                        .font(.footnote)
                                        .foregroundStyle(.secondary)
                                }
                                .buttonStyle(.plain)
                                if let error = entry.error {
                                    Label(error, systemImage: "exclamationmark.triangle")
                                        .font(.footnote)
                                        .foregroundStyle(.orange)
                                }
                                if entry.flock?.details != nil && entry.agents.isEmpty {
                                    Text("No agents running").foregroundStyle(.secondary)
                                }
                            }
                            .padding(.top, 8)
                        }
                    }
                }
                .padding(.horizontal)
                .padding(.bottom)
            }
        }
    }

    private func card(_ agent: AgentSummary, in entry: MachineFlockEntry, starred: Bool, now: Date) -> some View {
        let route = AgentRoute(machineId: entry.id, terminalId: agent.terminalId)
        return NavigationLink(value: route) {
            AgentCard(
                agent: agent, screen: previews.screens[route], workspace: entry.workspaceLabel(for: agent),
                followed: follows?.isFollowing(route) == true, starred: starred, now: now
            )
        }
        .buttonStyle(.plain)
        .opacity(entry.linkDown ? 0.5 : 1)
        .contextMenu {
            Button(starred ? "Unstar" : "Star", systemImage: starred ? "star.slash" : "star") { Task { await previews.toggleStar(route) } }
                .disabled(entry.flock?.link != .connected)
            menu(agent, route)
        }
        .onAppear { previews.appeared(route) }
        .onDisappear { previews.disappeared(route) }
    }
}

extension AgentGrid {
    /// No live preview: reading a shell needs a grant.
    private func terminalCard(_ terminal: TerminalSummary, in entry: MachineFlockEntry) -> some View {
        let route = AgentRoute(machineId: entry.id, terminalId: terminal.terminalId)
        return NavigationLink(value: route) {
            HStack(spacing: 6) {
                AgentKindLabel(kind: "terminal", iconOnly: true)
                VStack(alignment: .leading, spacing: 1) {
                    Text(terminal.displayTitle).font(.subheadline).lineLimit(1)
                    Text([terminal.workspaceLabel, entry.machine.label].compactMap { $0 }.joined(separator: " · "))
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                }
                Spacer(minLength: 0)
                Image(systemName: terminal.locked ? "lock.fill" : "lock.open")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .accessibilityLabel(terminal.locked ? "Locked" : "Unlocked")
            }
            .padding(10)
            .background(.fill.tertiary, in: RoundedRectangle(cornerRadius: 10))
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .opacity(entry.linkDown ? 0.5 : 1)
        .contextMenu {
            Button("Close pane", systemImage: "xmark.square", role: .destructive) { closePane(route) }
        }
    }
}

extension MachineFlockEntry {
    func gridAgents(starred: Set<AgentRoute>) -> (starred: [AgentSummary], rest: [AgentSummary]) {
        let isStarred = { (agent: AgentSummary) in starred.contains(AgentRoute(machineId: id, terminalId: agent.terminalId)) }
        return (agents.filter(isStarred), agents.filter { !isStarred($0) })
    }
}

private struct AgentCard: View {
    let agent: AgentSummary
    let screen: String?
    let workspace: String?
    let followed: Bool
    let starred: Bool
    let now: Date
    @Environment(\.dynamicTypeSize) private var typeSize

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            TerminalPreview(ansi: screen ?? "")
                .frame(height: starred ? 240 : 120)
                .clipShape(RoundedRectangle(cornerRadius: 10))
                .allowsHitTesting(false)
                .accessibilityHidden(true)
            HStack(alignment: .top, spacing: 6) {
                StatusIcon(state: agent.status).accessibilityHidden(true)
                VStack(alignment: .leading, spacing: 1) {
                    HStack(spacing: 4) {
                        if let kind = agent.kind {
                            AgentKindLabel(kind: kind, iconOnly: true)
                        }
                        if starred {
                            Image(systemName: "star.fill")
                                .font(.caption2)
                                .foregroundStyle(.yellow)
                                .accessibilityLabel("Starred")
                        }
                        if followed {
                            Image(systemName: "pin.fill")
                                .font(.caption2)
                                .foregroundStyle(.tint)
                                .accessibilityLabel("Followed")
                        }
                        Text(agent.displayTitle).font(.subheadline).lineLimit(typeSize.isAccessibilitySize ? 2 : 1)
                    }
                    HStack(spacing: 4) {
                        if let workspace {
                            Text(workspace)
                            Text(verbatim: "·")
                        }
                        StatusAge(agent: agent, now: now).fixedSize()
                    }
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                }
                if let left = agent.contextLeft {
                    Spacer(minLength: 0)
                    ContextRing(left: left).padding(.top, 3)
                }
            }
        }
        .contentShape(Rectangle())
        .accessibilityElement(children: .combine)
    }
}

/// Read-only: no scrolling, selection or gestures; it follows the bottom of the snapshot.
private struct TerminalPreview: UIViewRepresentable {
    let ansi: String

    func makeUIView(context: Context) -> GhosttyTerminalUIView {
        let view = GhosttyTerminalUIView(fontSize: 5)
        view.wraps = true
        view.isUserInteractionEnabled = false
        view.showsVerticalScrollIndicator = false
        view.showsHorizontalScrollIndicator = false
        return view
    }

    func updateUIView(_ view: GhosttyTerminalUIView, context: Context) {
        if !ansi.isEmpty {
            view.show(ansiSnapshot: ansi)
        }
    }
}
