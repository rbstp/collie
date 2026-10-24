import CollieCore
import SwiftUI

struct AgentInbox<Menu: View>: View {
    let entries: [MachineFlockEntry]
    let notice: String?
    let follows: FollowModel?
    let seen: SeenAgents
    let markUnseen: (AgentSummary, AgentRoute) -> Void
    let snoozed: SnoozedAgents
    let snooze: (AgentSummary, AgentRoute, Date?) -> Void
    @Binding var query: String
    @Binding var filter: InboxFilter
    @Binding var showsSnoozed: Bool
    @ViewBuilder let menu: (AgentSummary, AgentRoute) -> Menu
    @State private var snoozing: InboxItem?
    @State private var custom: InboxItem?
    @State private var customWake = Date.now

    var body: some View {
        let items = InboxItem.items(in: entries)
        TimelineView(InboxSchedule(wakes: snoozed.snoozes.values.map(\.wake))) { context in
            List {
                if let notice {
                    Label(notice, systemImage: "exclamationmark.triangle")
                        .font(.footnote)
                        .foregroundStyle(.orange)
                }
                if entries.isEmpty {
                    NoMachines()
                }
                inbox(items.filter { $0.matches(query) }, now: context.date)
            }
            .safeAreaInset(edge: .top, spacing: 0) {
                VStack(spacing: 0) {
                    ScrollView(.horizontal) {
                        GlassPicker(title: "Filter", selection: $filter,
                                    options: InboxFilter.allCases.map { ($0.title, $0) })
                            .padding(.horizontal)
                    }
                    .scrollIndicators(.hidden)
                    // Outside the scroll view: one that touches the bar's safe area insets its content, and the segments again.
                    .padding(.vertical, 8)
                    // Not .searchable: its field moves over the rows and then into the navigation bar when focused.
                    HStack {
                        Image(systemName: "magnifyingglass").foregroundStyle(.secondary)
                        TextField("Search agents", text: $query)
                            .submitLabel(.search)
                        if !query.isEmpty {
                            Button("Clear search", systemImage: "xmark.circle.fill") { query = "" }
                                .labelStyle(.iconOnly)
                                .foregroundStyle(.secondary)
                        }
                    }
                    .padding(.horizontal, 12)
                    .frame(minHeight: 44)
                    .glassEffect(.regular, in: .capsule)
                    .padding(.horizontal)
                    .padding(.bottom, 8)
                }
            }
        }
        .confirmationDialog("Snooze until", isPresented: Binding { snoozing != nil } set: { if !$0 { snoozing = nil } },
                            titleVisibility: .visible, presenting: snoozing) { item in
            let morning = SnoozedAgents.nextMorning(after: .now)
            Button("1 hour") { snooze(item.agent, item.route, .now.addingTimeInterval(3600)) }
            Button("4 hours") { snooze(item.agent, item.route, .now.addingTimeInterval(4 * 3600)) }
            Button(Calendar.current.isDateInToday(morning) ? "Today 8:00" : "Tomorrow 8:00") {
                snooze(item.agent, item.route, morning)
            }
            Button("Custom") {
                customWake = .now.addingTimeInterval(3600)
                custom = item
            }
        }
        .sheet(item: $custom) { item in
            NavigationStack {
                Form {
                    DatePicker("Wake at", selection: $customWake, in: Date.now..., displayedComponents: [.date, .hourAndMinute])
                }
                .navigationTitle("Snooze")
                .navigationBarTitleDisplayMode(.inline)
                .toolbar {
                    ToolbarItem(placement: .cancellationAction) {
                        Button("Cancel") { custom = nil }
                    }
                    ToolbarItem(placement: .confirmationAction) {
                        Button("Snooze") {
                            snooze(item.agent, item.route, customWake)
                            custom = nil
                        }
                    }
                }
            }
            .presentationDetents([.medium])
        }
    }

    @ViewBuilder
    private func inbox(_ items: [InboxItem], now: Date) -> some View {
        let (groups, asleep) = InboxItem.rows(items, filter: filter, seen: seen, snoozed: snoozed, now: now)
        ForEach(InboxSection.allCases, id: \.self) { section in
            if let rows = groups[section] {
                Section(section.title) {
                    ForEach(rows) { row($0, wake: nil, now: now) }
                }
            }
        }
        if !asleep.isEmpty {
            Section {
                if showsSnoozed {
                    ForEach(asleep, id: \.0.id) { row($0.0, wake: $0.1, now: now) }
                }
            } header: {
                Button {
                    showsSnoozed.toggle()
                } label: {
                    HStack {
                        Text("Snoozed (\(asleep.count))")
                        Spacer()
                        Image(systemName: "chevron.right")
                            .rotationEffect(.degrees(showsSnoozed ? 90 : 0))
                            .accessibilityHidden(true)
                    }
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .accessibilityValue(showsSnoozed ? "Expanded" : "Collapsed")
            }
        }
        if groups.isEmpty && (asleep.isEmpty || filter != .all) && (entries.contains { $0.flock?.details != nil } || !query.isEmpty || filter != .all) {
            Text(query.isEmpty && filter == .inactive ? "No inactive agents"
                 : query.isEmpty && filter == .all ? "No agents running" : "No matching agents")
                .foregroundStyle(.secondary)
        }
    }

    private func row(_ item: InboxItem, wake: Date?, now: Date) -> some View {
        let isSeen = seen.isSeen(item.agent, route: item.route)
        return NavigationLink(value: item.route) {
            InboxRow(item: item, followed: follows?.isFollowing(item.route) == true,
                     unseen: item.agent.status == .done && !isSeen, wake: wake, now: now)
        }
        .contextMenu {
            if wake != nil {
                Button("Wake now", systemImage: "alarm") { snooze(item.agent, item.route, nil) }
                    .tint(.primary)
                Divider()
            }
            if isSeen {
                Button("Mark unseen", systemImage: "circle.fill") { markUnseen(item.agent, item.route) }
                    .tint(.primary)
                Divider()
            }
            menu(item.agent, item.route)
        }
        .swipeActions(edge: .leading) {
            if item.agent.status != .blocked {
                Button("Snooze", systemImage: "moon.zzz") { snoozing = item }
                    .tint(.indigo)
            }
        }
        .opacity(item.linkDown ? 0.5 : 1)
    }
}

/// Every minute for the elapsed times, plus each wake time so a snoozed agent returns on time.
struct InboxSchedule: TimelineSchedule {
    let wakes: [Date]

    func entries(from startDate: Date, mode: TimelineScheduleMode) -> AnyIterator<Date> {
        var minute = startDate
        var wakes = wakes.filter { $0 > startDate }.sorted()
        return AnyIterator {
            if let wake = wakes.first, wake < minute {
                wakes.removeFirst()
                return wake
            }
            defer { minute += 60 }
            return minute
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

    static func rows(_ items: [InboxItem], filter: InboxFilter, seen: SeenAgents, snoozed: SnoozedAgents, now: Date)
        -> (active: [InboxSection: [InboxItem]], asleep: [(InboxItem, Date)])
    {
        let active = items.filter { snoozed.wake(of: $0.agent, route: $0.route, now: now) == nil && filter.includes($0, seen: seen, now: now) }
        let asleep = items.compactMap { item in snoozed.wake(of: item.agent, route: item.route, now: now).map { (item, $0) } }
            .sorted { ($0.1, $0.0.route.terminalId) < ($1.1, $1.0.route.terminalId) }
        return (InboxSection.grouped(active, now: now), asleep)
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
        case .idle, .unknown: return .inactive
        case .done:
            let age = now.timeIntervalSince1970 - TimeInterval(agent.inboxActivityMs) / 1000
            return age < inactiveAfter ? .done : .inactive
        }
    }

    /// Blocked agents first, then the most recent activity first.
    static func grouped(_ items: [InboxItem], now: Date) -> [InboxSection: [InboxItem]] {
        Dictionary(grouping: items) { of($0.agent, now: now) }.mapValues { rows in
            rows.sorted {
                ($0.agent.status == .blocked ? 0 : 1, UInt64.max - $0.agent.inboxActivityMs, $0.route.terminalId)
                    < ($1.agent.status == .blocked ? 0 : 1, UInt64.max - $1.agent.inboxActivityMs, $1.route.terminalId)
            }
        }
    }
}

private struct InboxRow: View {
    let item: InboxItem
    let followed: Bool
    let unseen: Bool
    let wake: Date?
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
                if let line = agent.lastPromptOrigin?.wakeLine {
                    Text("\(Image(systemName: "eye")) \(line)")
                        .font(.subheadline)
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                } else if let prompt = agent.lastPrompt {
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
                    if let pullRequest = agent.pullRequest {
                        PullRequestBadge(pullRequest: pullRequest, watch: agent.prWatch)
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
                if let wake {
                    let format: Date.FormatStyle = Calendar.current.isDate(wake, inSameDayAs: now)
                        ? .dateTime.hour().minute() : .dateTime.month(.abbreviated).day().hour().minute()
                    Text(wake.formatted(format))
                        .font(.caption.monospacedDigit())
                        .foregroundStyle(.secondary)
                        .accessibilityLabel("Snoozed until \(wake.formatted(date: .abbreviated, time: .shortened))")
                } else {
                    Text(Elapsed.compact(sinceMs: item.agent.inboxActivityMs, now: now))
                        .font(.caption.monospacedDigit())
                        .foregroundStyle(.secondary)
                        .accessibilityLabel("\(Elapsed.spoken(sinceMs: item.agent.inboxActivityMs, now: now)) ago")
                }
                if let left = agent.contextLeft {
                    ContextRing(left: left)
                }
            }
        }
    }
}

private extension AgentSummary {
    var inboxActivityMs: UInt64 {
        status == .working || status == .blocked || statusSinceMs == 0 ? activityMs : statusSinceMs
    }
}
