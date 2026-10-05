import CollieCore
import GhosttyTerminal
import SwiftUI

/// The Agents list as cards, each with the end of the agent's screen from `PreviewModel`.
struct AgentGrid<Menu: View>: View {
    let entries: [MachineFlockEntry]
    let previews: PreviewModel
    let notice: String?
    let approvalsCount: (MachineFlockEntry) -> Int
    let showsLink: Bool
    let follows: FollowModel?
    @ViewBuilder let menu: (AgentSummary, AgentRoute) -> Menu

    var body: some View {
        ScrollView {
            if let notice {
                Label(notice, systemImage: "exclamationmark.triangle")
                    .font(.footnote)
                    .foregroundStyle(.orange)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
            if entries.isEmpty {
                NoMachines()
            }
            LazyVGrid(columns: [GridItem(.adaptive(minimum: 150), spacing: 12)], alignment: .leading, spacing: 16) {
                ForEach(entries) { entry in
                    Section {
                        ForEach(entry.agents, id: \.terminalId) { agent in
                            let route = AgentRoute(machineId: entry.id, terminalId: agent.terminalId)
                            NavigationLink(value: route) {
                                AgentCard(
                                    agent: agent, screen: previews.screens[route], workspace: entry.workspaceLabel(for: agent),
                                    machine: entry.machine.label, followed: follows?.isFollowing(route) == true
                                )
                            }
                            .buttonStyle(.plain)
                            .opacity(entry.linkDown ? 0.5 : 1)
                            .contextMenu { menu(agent, route) }
                            .onAppear { previews.appeared(route) }
                            .onDisappear { previews.disappeared(route) }
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
}

private struct AgentCard: View {
    let agent: AgentSummary
    let screen: String?
    let workspace: String?
    let machine: String
    let followed: Bool

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            TerminalPreview(ansi: screen ?? "")
                .frame(height: 120)
                .clipShape(RoundedRectangle(cornerRadius: 10))
                .allowsHitTesting(false)
                .accessibilityHidden(true)
            HStack(alignment: .top, spacing: 6) {
                StatusIcon(state: agent.status)
                VStack(alignment: .leading, spacing: 1) {
                    HStack(spacing: 4) {
                        if followed {
                            Image(systemName: "pin.fill")
                                .font(.caption2)
                                .foregroundStyle(.tint)
                                .accessibilityLabel("Followed")
                        }
                        Text(agent.displayTitle).font(.subheadline).lineLimit(1)
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
