import CollieCore
import SwiftUI

struct AgentInbox<Menu: View>: View {
    let entries: [MachineFlockEntry]
    let notice: String?
    let reconnect: (MachineFlockEntry) -> Void
    let follows: FollowModel?
    @ViewBuilder let menu: (AgentSummary, AgentRoute) -> Menu

    var body: some View {
        let items = InboxItem.items(in: entries)
        TimelineView(.periodic(from: .now, by: 60)) { context in
            List {
                if let notice {
                    Label(notice, systemImage: "exclamationmark.triangle")
                        .font(.footnote)
                        .foregroundStyle(.orange)
                }
                if entries.isEmpty {
                    NoMachines()
                }
                ForEach(entries.filter { $0.error != nil }) { entry in
                    Button {
                        reconnect(entry)
                    } label: {
                        Label("\(entry.machine.label): \(entry.error ?? "")", systemImage: "exclamationmark.triangle")
                            .font(.footnote)
                            .foregroundStyle(.orange)
                    }
                    .buttonStyle(.plain)
                }
                inbox(items, now: context.date)
            }
        }
    }

    @ViewBuilder
    private func inbox(_ items: [InboxItem], now: Date) -> some View {
        let groups = InboxSection.grouped(items, now: .now)
        ForEach(InboxSection.allCases, id: \.self) { section in
            if let rows = groups[section] {
                Section(section.title) {
                    ForEach(rows) { item in
                        NavigationLink(value: item.route) {
                            InboxRow(item: item, followed: follows?.isFollowing(item.route) == true, now: now)
                        }
                        .contextMenu { menu(item.agent, item.route) }
                        .opacity(item.linkDown ? 0.5 : 1)
                    }
                }
            }
        }
        if items.isEmpty && entries.contains(where: { $0.flock?.details != nil }) {
            Text("No agents running").foregroundStyle(.secondary)
        }
    }
}

struct InboxItem: Identifiable, Equatable {
    let agent: AgentSummary
    let route: AgentRoute
    let workspace: String?
    let machine: String
    let linkDown: Bool

    var id: AgentRoute { route }

    static func items(in entries: [MachineFlockEntry]) -> [InboxItem] {
        entries.flatMap { entry in
            entry.agents.map {
                InboxItem(
                    agent: $0, route: AgentRoute(machineId: entry.id, terminalId: $0.terminalId),
                    workspace: entry.workspaceLabel(for: $0), machine: entry.machine.label, linkDown: entry.linkDown
                )
            }
        }
    }
}

enum InboxSection: CaseIterable {
    case working
    case done
    case archived

    static let archiveAfter: TimeInterval = 24 * 3600

    var title: String {
        switch self {
        case .working: "Working"
        case .done: "Done"
        case .archived: "Archived"
        }
    }

    static func of(_ agent: AgentSummary, now: Date) -> InboxSection {
        switch agent.status {
        case .working, .blocked: return .working
        case .done: return .done
        case .idle, .unknown:
            let age = now.timeIntervalSince1970 - TimeInterval(agent.activityMs) / 1000
            return age < archiveAfter ? .done : .archived
        }
    }

    /// Blocked agents first, then the most recent activity first.
    static func grouped(_ items: [InboxItem], now: Date) -> [InboxSection: [InboxItem]] {
        Dictionary(grouping: items) { of($0.agent, now: now) }.mapValues { rows in
            rows.sorted {
                ($0.agent.status == .blocked ? 0 : 1, UInt64.max - $0.agent.activityMs, $0.route.terminalId)
                    < ($1.agent.status == .blocked ? 0 : 1, UInt64.max - $1.agent.activityMs, $1.route.terminalId)
            }
        }
    }
}

private struct InboxRow: View {
    let item: InboxItem
    let followed: Bool
    let now: Date

    var body: some View {
        let agent = item.agent
        HStack(alignment: .firstTextBaseline, spacing: 12) {
            StatusIcon(state: agent.status)
                .alignmentGuide(.firstTextBaseline) { $0[VerticalAlignment.center] + 6 }
            VStack(alignment: .leading, spacing: 3) {
                HStack(alignment: .firstTextBaseline, spacing: 4) {
                    if followed {
                        Image(systemName: "pin.fill")
                            .font(.caption)
                            .foregroundStyle(.tint)
                            .accessibilityLabel("Followed")
                    }
                    Text(agent.lastLine ?? agent.displayTitle).lineLimit(2)
                }
                if let prompt = agent.lastPrompt {
                    Text("You: \(prompt)")
                        .font(.subheadline)
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                }
                HStack(spacing: 6) {
                    if let workspace = item.workspace {
                        Text(workspace)
                            .font(.caption2.weight(.medium))
                            .padding(.horizontal, 6)
                            .padding(.vertical, 1)
                            .background(.quaternary, in: Capsule())
                            .layoutPriority(1)
                    }
                    if let kind = agent.kind {
                        AgentKindLabel(kind: kind, iconOnly: true)
                    }
                    Text(item.machine)
                }
                .lineLimit(1)
                .font(.caption)
                .foregroundStyle(.secondary)
            }
            Spacer()
            VStack(alignment: .trailing, spacing: 6) {
                Text(Elapsed.compact(sinceMs: item.agent.activityMs, now: now))
                    .font(.caption.monospacedDigit())
                    .foregroundStyle(.secondary)
                    .accessibilityLabel("active \(Elapsed.spoken(sinceMs: item.agent.activityMs, now: now)) ago")
                if let left = agent.contextLeft {
                    ContextRing(left: left)
                }
            }
        }
    }
}
