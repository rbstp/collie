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
    @Binding var searching: Bool
    @Binding var filter: InboxFilter
    @Binding var showsSnoozed: Bool
    let margin: CGFloat?
    let refresh: () async -> Void
    @ViewBuilder let menu: (AgentSummary, AgentRoute) -> Menu
    @State private var snoozing: InboxItem?
    @State private var custom: InboxItem?
    @State private var customWake = Date.now
    @FocusState private var searchFocused: Bool
    @ScaledMetric(relativeTo: .subheadline) private var filterFontSize = 15
    @State private var filterSpace: CGFloat = 0
    @State private var filterAtEnd = false
    @State private var filterClips = (leading: false, trailing: false)
    @State private var filterPosition = ScrollPosition(edge: .leading)

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
            .listSectionSpacing(.compact)
            .modifier(PagerListMargin(edge: .trailing, margin: margin))
        }
        // Before the inset: the filter row's own scroll view would take a refresh control, and a drag beside the filters pulled the row down.
        .refreshable { await refresh() }
        .safeAreaInset(edge: .top, spacing: 0) {
            Group {
                if searching || !query.isEmpty {
                    // Not .searchable: its field moves over the rows and then into the navigation bar when focused.
                    HStack(spacing: 8) {
                        HStack {
                            Image(systemName: "magnifyingglass").foregroundStyle(.secondary)
                            TextField("Search agents", text: $query, prompt: Text("Search agents").foregroundStyle(.metadata))
                                .submitLabel(.search)
                                .focused($searchFocused)
                                .onAppear { searchFocused = searching }
                        }
                        .padding(.horizontal, 12)
                        .frame(minHeight: 44)
                        .glassEffect(.regular, in: .capsule)
                        closeSearchButton
                    }
                    .padding(.horizontal)
                } else {
                    filterRow
                }
            }
            // Outside the scroll view: one that touches the bar's safe area insets its content, and the segments again.
            .padding(.vertical, 8)
            .onChange(of: searchFocused) { _, focused in
                if !focused && query.isEmpty { searching = false }
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

    private var filterRow: some View {
        let segments = filterSegments
        return ScrollView(.horizontal) {
            GlassPicker(title: "Filter", selection: $filter, options: InboxFilter.allCases.map { ($0.title, $0) })
                .padding(.horizontal, InboxFilter.rowInset)
        }
        .scrollIndicators(.hidden)
        .scrollPosition($filterPosition)
        // A scroll before the segments have a size is lost, which left a filter chosen earlier past the fade.
        .onScrollGeometryChange(for: CGFloat.self) { $0.contentSize.width } action: { _, _ in revealFilter() }
        .onScrollGeometryChange(for: Bool.self) { $0.contentOffset.x > 1 } action: { filterClips.leading = $1 }
        .onScrollGeometryChange(for: Bool.self) { $0.contentOffset.x + $0.containerSize.width < $0.contentSize.width - 1 } action: {
            filterClips.trailing = $1
        }
        // Past an edge with more filters, the track fades out; the row ends between two labels, never across one.
        .mask {
            HStack(spacing: 0) {
                LinearGradient(colors: [.clear, .black], startPoint: .leading, endPoint: .trailing).frame(width: filterClips.leading ? 18 : 0)
                Color.black
                LinearGradient(colors: [.black, .clear], startPoint: .leading, endPoint: .trailing).frame(width: filterClips.trailing ? 18 : 0)
            }
        }
        .frame(width: InboxFilter.rowWidth(segments: segments, available: filterSpace, atEnd: filterAtEnd))
        .frame(maxWidth: .infinity, alignment: .leading)
        .onGeometryChange(for: CGFloat.self) { $0.size.width } action: {
            filterSpace = $0
            revealFilter()
        }
        .onChange(of: filter) { revealFilter() }
        .onAppear(perform: revealFilter)
    }

    private var filterSegments: [CGFloat] {
        let font = UIFont.systemFont(ofSize: filterFontSize, weight: .semibold)
        return InboxFilter.allCases.map { GlassSegment.width($0.title, font: font) }
    }

    /// A filter the row would hide, such as one chosen before a search, shows by scrolling the row to its end.
    private func revealFilter() {
        guard filterSpace > 0, let index = InboxFilter.allCases.firstIndex(of: filter) else { return }
        let segments = filterSegments
        let end = segments[...index].reduce(InboxFilter.rowInset, +) - GlassSegment.padding
        filterAtEnd = end > InboxFilter.rowWidth(segments: segments, available: filterSpace, atEnd: false)
        filterPosition.scrollTo(edge: filterAtEnd ? .trailing : .leading)
    }

    private var closeSearchButton: some View {
        Button("Close search", systemImage: "xmark") {
            query = ""
            searching = false
        }
        .labelStyle(.iconOnly)
        .font(.system(size: 16, weight: .medium))
        .foregroundStyle(.primary)
        .frame(width: 38, height: 38)
        .glassEffect(.regular.interactive(), in: .circle)
        .contentShape(Circle().inset(by: -3))
    }

    @ViewBuilder
    private func inbox(_ items: [InboxItem], now: Date) -> some View {
        // The field covers the filter row, so an open search looks through every section.
        let filter = searching || !query.isEmpty ? .all : self.filter
        let (groups, asleep) = InboxItem.rows(items, filter: filter, snoozed: snoozed, now: now)
        ForEach(InboxSection.allCases, id: \.self) { section in
            if let rows = groups[section] {
                let content = ForEach(rows) { row($0, wake: nil, now: now) }
                // A filtered inbox shows one group, which the selected filter already names.
                if filter == .all {
                    Section {
                        content
                    } header: {
                        Text(section.title).font(.footnote.weight(.semibold)).textCase(nil)
                    }
                } else {
                    Section { content }
                        // Without it, the header's room stayed as a blank band under the filters.
                        .listSectionMargins(.top, 16)
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
                    .font(.footnote.weight(.semibold))
                    .textCase(nil)
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
            InboxRow(item: item, showsMachine: entries.count > 1, followed: follows?.isFollowing(item.route) == true,
                     unseen: item.agent.status == .done && !isSeen, wake: wake, now: now)
        }
        // No disclosure chevron, so the text keeps the width, as in Mail.
        .navigationLinkIndicatorVisibility(.hidden)
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

    static func rows(_ items: [InboxItem], filter: InboxFilter, snoozed: SnoozedAgents, now: Date)
        -> (active: [InboxSection: [InboxItem]], asleep: [(InboxItem, Date)])
    {
        let active = items.filter { snoozed.wake(of: $0.agent, route: $0.route, now: now) == nil && filter.includes($0, now: now) }
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
    case all, needsYou, working, done, inactive

    var section: InboxSection? {
        switch self {
        case .all: nil
        case .needsYou: .needsYou
        case .working: .working
        case .done: .done
        case .inactive: .inactive
        }
    }

    var title: String { section?.title ?? "All" }

    static let rowInset: CGFloat = 16

    /// The filter row's width in `available` points: if the filters overflow, it ends in the padding
    /// before a label it would cut, or after one when scrolled to the end, so no label shows in part.
    static func rowWidth(segments: [CGFloat], available: CGFloat, atEnd: Bool) -> CGFloat {
        let content = segments.reduce(2 * rowInset, +)
        guard content > available else { return available }
        var x = rowInset
        let labels = segments.map { width in
            defer { x += width }
            return (start: x + GlassSegment.padding, end: x + width - GlassSegment.padding)
        }
        let edge = atEnd ? content - available : available
        guard let cut = labels.first(where: { $0.start < edge && edge < $0.end }) else { return available }
        return atEnd ? content - cut.end : cut.start
    }

    func includes(_ item: InboxItem, now: Date) -> Bool {
        section.map { InboxSection.of(item.agent, now: now) == $0 } ?? true
    }
}

enum InboxSection: CaseIterable {
    case needsYou
    case working
    case done
    case inactive

    static let inactiveAfter: TimeInterval = 24 * 3600

    var title: String {
        switch self {
        case .needsYou: "Needs you"
        case .working: "Working"
        case .done: "Done"
        case .inactive: "Inactive"
        }
    }

    static func of(_ agent: AgentSummary, now: Date) -> InboxSection {
        switch agent.status {
        case .blocked: return .needsYou
        case .working: return .working
        case .idle, .unknown: return .inactive
        case .done:
            let age = now.timeIntervalSince1970 - TimeInterval(agent.inboxActivityMs) / 1000
            return age < inactiveAfter ? .done : .inactive
        }
    }

    /// The most recent activity first.
    static func grouped(_ items: [InboxItem], now: Date) -> [InboxSection: [InboxItem]] {
        Dictionary(grouping: items) { of($0.agent, now: now) }.mapValues { rows in
            rows.sorted {
                (UInt64.max - $0.agent.inboxActivityMs, $0.route.terminalId) < (UInt64.max - $1.agent.inboxActivityMs, $1.route.terminalId)
            }
        }
    }
}

private struct InboxRow: View {
    let item: InboxItem
    let showsMachine: Bool
    let followed: Bool
    let unseen: Bool
    let wake: Date?
    let now: Date
    @ScaledMetric(relativeTo: .subheadline) private var lift: CGFloat = 5

    var body: some View {
        let agent = item.agent
        HStack(alignment: .firstTextBaseline, spacing: 12) {
            StatusIcon(state: agent.status)
                .alignmentGuide(.firstTextBaseline) { $0[VerticalAlignment.center] + lift }
            VStack(alignment: .leading, spacing: 2) {
                HStack(alignment: .firstTextBaseline, spacing: 6) {
                    Text(agent.displayTitle).font(.subheadline.weight(.semibold)).lineLimit(1)
                    Spacer(minLength: 0)
                    Group {
                        if let wake {
                            let format: Date.FormatStyle = Calendar.current.isDate(wake, inSameDayAs: now)
                                ? .dateTime.hour().minute() : .dateTime.month(.abbreviated).day().hour().minute()
                            Label(wake.formatted(format), systemImage: "moon.zzz")
                                .labelStyle(KindLabelStyle())
                                .accessibilityLabel("Snoozed until \(wake.formatted(date: .abbreviated, time: .shortened))")
                        } else {
                            Text(Elapsed.compact(sinceMs: item.agent.inboxActivityMs, now: now))
                                .accessibilityLabel("\(Elapsed.spoken(sinceMs: item.agent.inboxActivityMs, now: now)) ago")
                        }
                    }
                    .font(.caption)
                    .monospacedDigit()
                    .foregroundStyle(.metadata)
                    .fixedSize()
                    if unseen {
                        Image(systemName: "circle.fill")
                            .font(.system(size: 8))
                            .foregroundStyle(.primary)
                            .alignmentGuide(.firstTextBaseline) { $0[VerticalAlignment.center] + 4 }
                            .accessibilityLabel("Unseen completion")
                    }
                }
                Group {
                    if let lastLine = agent.lastLine {
                        Text(lastLine)
                    } else if let line = agent.lastPromptOrigin?.wakeLine {
                        Text("\(Image(systemName: "eye")) \(line)")
                    } else if let prompt = agent.lastPrompt {
                        Text("You: \(prompt)")
                    }
                }
                .font(.footnote)
                .foregroundStyle(.metadata)
                .lineLimit(1)
                HStack(spacing: 4) {
                    if let kind = agent.kind {
                        AgentKindLabel(kind: kind, iconOnly: true)
                    }
                    if followed {
                        Image(systemName: "pin.fill")
                            .foregroundStyle(.tint)
                            .accessibilityLabel("Followed")
                    }
                    if let workspace = item.workspace {
                        Text(workspace).layoutPriority(1)
                        if showsMachine || item.stale {
                            Text(verbatim: "·").fixedSize()
                        }
                    }
                    if showsMachine {
                        Text(item.machine)
                    }
                    if item.stale {
                        Text(item.linkDown ? "Offline · cached" : "Cached")
                    }
                    if let pullRequest = agent.pullRequest {
                        PullRequestBadge(pullRequest: pullRequest, watch: agent.prWatch)
                    }
                    Spacer(minLength: 0)
                    if let left = agent.contextLeft {
                        ContextRing(left: left, font: .caption2, aligned: true)
                    }
                }
                .lineLimit(1)
                .font(.caption)
                .foregroundStyle(.metadata)
            }
        }
    }
}

private extension AgentSummary {
    var inboxActivityMs: UInt64 {
        status == .working || status == .blocked || statusSinceMs == 0 ? activityMs : statusSinceMs
    }
}
