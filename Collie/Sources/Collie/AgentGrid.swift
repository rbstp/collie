import CollieCore
import GhosttyTerminal
import SwiftUI

struct AgentGrid<Menu: View>: View {
    let entries: [MachineFlockEntry]
    let previews: PreviewModel
    let notice: String?
    let approvalsCount: (MachineFlockEntry) -> Int
    let showsLink: Bool
    let follows: FollowModel?
    @ViewBuilder let menu: (AgentSummary, AgentRoute) -> Menu
    @Environment(\.dynamicTypeSize) private var typeSize

    var body: some View {
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
                        ForEach(agents.starred, id: \.terminalId) { card($0, in: entry, starred: true) }
                        if !agents.rest.isEmpty {
                            LazyVGrid(
                                columns: typeSize.isAccessibilitySize ? [GridItem(.flexible())] : [GridItem(.adaptive(minimum: 150), spacing: 12)],
                                alignment: .leading, spacing: 16
                            ) {
                                ForEach(agents.rest, id: \.terminalId) { card($0, in: entry, starred: false) }
                            }
                        }
                    } header: {
                        VStack(alignment: .leading, spacing: 6) {
                            MachineHeader(entry: entry, approvalsCount: approvalsCount(entry), showsLink: showsLink)
                                .font(.footnote)
                                .foregroundStyle(.secondary)
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

    private func card(_ agent: AgentSummary, in entry: MachineFlockEntry, starred: Bool) -> some View {
        let route = AgentRoute(machineId: entry.id, terminalId: agent.terminalId)
        return NavigationLink(value: route) {
            AgentCard(
                agent: agent, screen: previews.screens[route], workspace: entry.workspaceLabel(for: agent),
                machine: entry.machine.label, followed: follows?.isFollowing(route) == true, starred: starred
            )
        }
        .buttonStyle(.plain)
        .opacity(entry.linkDown ? 0.5 : 1)
        .contextMenu {
            Button(starred ? "Unstar" : "Star", systemImage: starred ? "star.slash" : "star") { previews.toggleStar(route) }
            menu(agent, route)
        }
        .onAppear { previews.appeared(route) }
        .onDisappear { previews.disappeared(route) }
    }
}

extension MachineFlockEntry {
    /// Starred agents first, both parts in flock order.
    func gridAgents(starred: Set<AgentRoute>) -> (starred: [AgentSummary], rest: [AgentSummary]) {
        let isStarred = { (agent: AgentSummary) in starred.contains(AgentRoute(machineId: id, terminalId: agent.terminalId)) }
        return (agents.filter(isStarred), agents.filter { !isStarred($0) })
    }
}

private struct AgentCard: View {
    let agent: AgentSummary
    let screen: String?
    let workspace: String?
    let machine: String
    let followed: Bool
    let starred: Bool
    @Environment(\.dynamicTypeSize) private var typeSize

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            TerminalPreview(ansi: screen ?? "")
                .frame(height: starred ? 240 : 120)
                .clipShape(RoundedRectangle(cornerRadius: 10))
                .allowsHitTesting(false)
                .accessibilityHidden(true)
            HStack(alignment: .top, spacing: 6) {
                StatusIcon(state: agent.status)
                VStack(alignment: .leading, spacing: 1) {
                    HStack(spacing: 4) {
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
                    Text([workspace, machine].compactMap { $0 }.joined(separator: " · "))
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
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
