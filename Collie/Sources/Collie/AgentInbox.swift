import CollieCore
import SwiftUI

struct AgentInbox<Menu: View>: View {
    let entries: [MachineFlockEntry]
    let notice: String?
    let reconnect: (MachineFlockEntry) -> Void
    let follows: FollowModel?
    let seen: SeenAgents
    let markUnseen: (AgentSummary, AgentRoute) -> Void
    @ViewBuilder let menu: (AgentSummary, AgentRoute) -> Menu
    @State private var query = ""
    @State private var filter = InboxFilter.all

    var body: some View {
        let items = InboxItem.items(in: entries)
        VStack(spacing: 0) {
            ScrollView(.horizontal) {
                HStack(spacing: 8) {
                    ForEach(InboxFilter.allCases, id: \.self) { option in
                        Button(option.title) { filter = option }
                            .buttonStyle(.bordered)
                            .tint(filter == option ? .accentColor : .secondary)
                    }
                }
                .padding(.horizontal)
                .padding(.vertical, 8)
            }
            .scrollIndicators(.hidden)
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
                    inbox(items.filter { filter.includes($0, seen: seen, now: context.date) && $0.matches(query) }, now: context.date)
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
                }
            }
        }
        .searchable(text: $query, prompt: "Search agents")
    }

    @ViewBuilder
    private func inbox(_ items: [InboxItem], now: Date) -> some View {
        let groups = InboxSection.grouped(items, now: now)
        ForEach(InboxSection.allCases, id: \.self) { section in
            if let rows = groups[section] {
                Section(section.title) {
                    ForEach(rows) { item in
                        NavigationLink(value: item.route) {
                            InboxRow(item: item, followed: follows?.isFollowing(item.route) == true,
                                     unseen: item.agent.status == .done && !seen.isSeen(item.agent, route: item.route), now: now)
                        }
                        .contextMenu {
                            if seen.isSeen(item.agent, route: item.route) {
                                Button("Mark unseen", systemImage: "circle.fill") { markUnseen(item.agent, item.route) }
                                Divider()
                            }
                            menu(item.agent, item.route)
                        }
                        .opacity(item.linkDown ? 0.5 : 1)
                    }
                }
            }
        }
        if items.isEmpty && (entries.contains { $0.flock?.details != nil } || !query.isEmpty || filter != .all) {
            Text(query.isEmpty && filter == .inactive ? "No agents inactive for 24 hours"
                 : query.isEmpty && filter == .all ? "No agents running" : "No matching agents")
                .foregroundStyle(.secondary)
        }
    }
}

struct InboxItem: Identifiable, Equatable {
    let agent: AgentSummary
    let route: AgentRoute
    let workspace: String?
    let machine: String
    let linkDown: Bool
    let stale: Bool

    var id: AgentRoute { route }

    static func items(in entries: [MachineFlockEntry]) -> [InboxItem] {
        entries.flatMap { entry in
            entry.agents.map {
                InboxItem(
                    agent: $0, route: AgentRoute(machineId: entry.id, terminalId: $0.terminalId),
                    workspace: entry.workspaceLabel(for: $0), machine: entry.machine.label,
                    linkDown: entry.linkDown || entry.error != nil, stale: entry.error != nil || entry.flock?.link != .connected
                )
            }
        }
    }

    func matches(_ query: String) -> Bool {
        let query = query.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !query.isEmpty else { return true }
        return [agent.name, agent.title, agent.displayTitle, workspace, machine, agent.kind]
            .compactMap { $0 }.contains { $0.localizedStandardContains(query) }
    }
}

enum InboxFilter: CaseIterable {
    case all, needsAttention, working, inactive

    var title: String {
        switch self {
        case .all: "All"
        case .needsAttention: "Needs attention"
        case .working: "Working"
        case .inactive: "Inactive"
        }
    }

    func includes(_ item: InboxItem, seen: SeenAgents, now: Date) -> Bool {
        switch self {
        case .all: true
        case .needsAttention:
            item.agent.status == .blocked || (item.agent.status == .done && !seen.isSeen(item.agent, route: item.route))
        case .working: InboxSection.of(item.agent, now: now) == .working
        case .inactive: InboxSection.of(item.agent, now: now) == .inactive
        }
    }
}

enum InboxSection: CaseIterable {
    case working
    case done
    case inactive

    static let inactiveAfter: TimeInterval = 24 * 3600

    var title: String {
        switch self {
        case .working: "Working"
        case .done: "Done"
        case .inactive: "Inactive"
        }
    }

    static func of(_ agent: AgentSummary, now: Date) -> InboxSection {
        switch agent.status {
        case .working, .blocked: return .working
        case .done, .idle, .unknown:
            let age = now.timeIntervalSince1970 - TimeInterval(agent.activityMs) / 1000
            return age < inactiveAfter ? .done : .inactive
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
    let unseen: Bool
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
                    if item.stale {
                        Text(item.linkDown ? "Offline · cached" : "Cached")
                    }
                }
                .lineLimit(1)
                .font(.caption)
                .foregroundStyle(.secondary)
            }
            Spacer()
            VStack(alignment: .trailing, spacing: 6) {
                if unseen {
                    Image(systemName: "circle.fill")
                        .font(.caption2)
                        .foregroundStyle(.tint)
                        .accessibilityLabel("Unseen completion")
                }
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
