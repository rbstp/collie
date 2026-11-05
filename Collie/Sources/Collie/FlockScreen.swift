import CollieCore
import SwiftUI

struct FlockScreen: View {
    let core: (any FlockCore)?
    let machines: [Machine]
    let approvals: ApprovalsModel?
    let follows: FollowModel?
    @Binding var opening: AgentRoute?
    var persistsState = true
    var tailnetStarting = false
    var reselected = 0
    var navigationChanged: () -> Void = {}
    var viewing: (AgentRoute?) -> Void = { _ in }
    @State private var model = FlockModel()
    @State private var path: [AgentRoute] = []
    @State private var newTask = false
    @State private var previews = PreviewModel()
    @State private var layout = DevicePrefs.load(from: DevicePrefs.file).agentsLayout
    @State private var seen = SeenAgents.load(from: SeenAgents.file)
    @State private var snoozed = SnoozedAgents.load(from: SnoozedAgents.file)
    @State private var gridOrder = GridOrder.load(from: GridOrder.file)
    @State private var showsUsage = false
    @State private var listMargin: CGFloat?
    @State private var usageRefresh = 0
    @State private var dragging: GridDrag?
    @State private var inboxQuery = ""
    @State private var inboxFilter = InboxFilter.all
    @State private var inboxSearching = false
    @State private var showsSnoozed = false
    /// The order the layout showed when the session opened; nil follows the layout as it is now.
    @State private var swipeOrder: [AgentRoute]?
    @Environment(\.scenePhase) private var scenePhase
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        NavigationStack(path: $path) {
            let pagerLocked = dragging != nil || (layout == .inbox && !showsUsage)
            TabView(selection: $showsUsage) {
                agents.background(GridPageLock(locked: pagerLocked) { listMargin = $0 }).tag(false)
                List { UsageSections(entries: model.entries, core: core, refresh: usageRefresh) }
                    .modifier(PagerListMargin(edge: .leading, margin: listMargin))
                    .refreshable { await refresh() }
                    // Off screen, the Agents page is not laid out on a rotation, and Usage kept the other orientation's margin.
                    .background(GridPageLock(locked: pagerLocked) { listMargin = $0 })
                    .tag(true)
            }
            .tabViewStyle(.page(indexDisplayMode: .never))
            // The pager clips its pages to its frame, so it spans the bars for the pages to run under them.
            .ignoresSafeArea(.container)
            .scrollEdgeEffectHidden(true, for: [.top, .bottom])
            // UIKit's top scroll edge effect follows the pager, not the pages' own scroll views.
            .overlay(alignment: .top) {
                GeometryReader { geometry in
                    let background = Color(.systemGroupedBackground)
                    let height = geometry.safeAreaInsets.top + 12
                    // Solid under the whole bar: content fading out beside its glass controls showed through as a ghost.
                    LinearGradient(stops: [.init(color: background, location: geometry.safeAreaInsets.top / height),
                                           .init(color: background.opacity(0), location: 1)],
                                   startPoint: .top, endPoint: .bottom)
                        .frame(height: height)
                        .offset(y: -geometry.safeAreaInsets.top)
                }
                .allowsHitTesting(false)
            }
            .springLoadingBehavior(.disabled)
            .animation(reduceMotion ? nil : .smooth(duration: 0.3), value: showsUsage)
            .navigationTitle("Agents")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar(removing: .title)
            .toolbar {
                ToolbarItem(placement: .topBarLeading) {
                    if tailnetStarting {
                        HStack(spacing: 6) {
                            ProgressView().controlSize(.small)
                            Text("Connecting…").font(.headline)
                        }
                    } else {
                        ViewSwitch(showsUsage: $showsUsage).fixedSize()
                    }
                }
                .sharedBackgroundVisibility(.hidden)
                if !showsUsage {
                    ToolbarItem(placement: .primaryAction) {
                        Menu("Layout", systemImage: layout.icon) {
                            Picker("Layout", selection: $layout) {
                                ForEach(AgentsLayout.allCases, id: \.self) { Label($0.label, systemImage: $0.icon) }
                            }
                            .tint(.primary)
                        }
                    }
                    // The open field has its own close button.
                    if layout == .inbox && !inboxSearching && inboxQuery.isEmpty {
                        ToolbarItem(placement: .primaryAction) {
                            Button("Search", systemImage: "magnifyingglass") { inboxSearching = true }
                        }
                    }
                }
                ToolbarItem(placement: .primaryAction) {
                    Button("New task", systemImage: "plus") { newTask = true }
                        .disabled(core == nil || machines.isEmpty || !persistsState)
                }
            }
            .closeConfirmation($model.close, archiving: model.archiving, archiveNotice: model.archiveNotice) {
                if let core, await model.performClose(core: core) {
                    await model.refresh(core: core, machines: machines)
                }
            }
            .navigationDestination(for: AgentRoute.self) { route in
                if let core {
                    AgentScreen(
                        core: core, route: route, persistsState: persistsState, approvals: approvals, follows: follows,
                        machineLabel: machines.first { $0.id == route.machineId }?.label, showsMachine: machines.count > 1,
                        neighbor: { offset in
                            // model.entries stops refreshing under a pushed agent; the core's cache follows events.
                            let entries = machines.map { MachineFlockEntry(machine: $0, flock: core.cachedFlock(machineId: $0.id)) }
                            let starred = Set(entries.flatMap { entry in (entry.flock?.starred ?? []).map { AgentRoute(machineId: entry.id, terminalId: $0) } })
                            let order = swipeOrder ?? layoutOrder(entries, starred: starred, filtered: false)
                            return FlockOrder.neighbor(of: route, offset: offset, in: order, among: entries)
                        },
                        switchAgent: { next in
                            // Replaces the top route, so Back still returns to this list.
                            if path.last == route { path[path.count - 1] = next }
                        },
                        opened: { agent in
                            seen.markOpened(agent, route: route, nowMs: Date().unixMs)
                            if persistsState { seen.save(to: SeenAgents.file) }
                            if snoozed.wakeNow(route), persistsState { snoozed.save(to: SnoozedAgents.file) }
                        },
                        archived: { model.showArchiveNotice($0) }
                    )
                    // A replaced route must get its own model, not keep the previous agent's.
                    .id(route)
                }
            }
            .sheet(isPresented: $newTask) {
                if let core {
                    NewTaskSheet(
                        core: core, machines: machines,
                        preferredMachineId: model.entries.first { $0.flock?.link == .connected }?.id,
                        planUsage: model.entries.reduce(into: [:]) { usage, entry in usage[entry.id] = entry.flock?.planUsage }
                    ) { path.append($0) }
                }
            }
            .onAppear {
                if !persistsState {
                    seen = SeenAgents()
                    snoozed = SnoozedAgents()
                    gridOrder = GridOrder()
                    layout = .grid
                }
            }
            .task(id: machines) {
                // Events keep the cache current; no event lists shells, and an older collied sends no title change.
                var tick = 0
                // Offline with no error is also a node that never reaches Running (logged out, no network).
                let nodeStart = ContinuousClock.now + .seconds(20)
                while !Task.isCancelled {
                    let snapshot = tick % 20 == 0
                    await model.refresh(core: core, machines: machines, snapshot: snapshot)
                    if snapshot && persistsState {
                        AgentDrafts.prune(model.entries, file: AgentDrafts.file)
                        if core != nil {
                            let previous = seen.markers.count
                            seen.prune(model.entries)
                            if seen.markers.count != previous { seen.save(to: SeenAgents.file) }
                            let snoozes = snoozed.snoozes.count
                            snoozed.prune(model.entries, now: .now)
                            if snoozed.snoozes.count != snoozes { snoozed.save(to: SnoozedAgents.file) }
                        }
                    }
                    follows?.sync()
                    tick += 1
                    let next = ContinuousClock.now + .seconds(3)
                    // While a machine connects, its list shows as soon as it reaches the cache; these reads stay local.
                    while model.entries.contains(where: { $0.connecting(nodeStarting: ContinuousClock.now < nodeStart) }),
                        ContinuousClock.now < next, !Task.isCancelled
                    {
                        try? await Task.sleep(for: .milliseconds(250))
                        await model.refresh(core: core, machines: machines, snapshot: false, cacheOnly: true)
                    }
                    try? await Task.sleep(until: next)
                }
            }
            .onChange(of: model.entries, initial: true) { _, entries in
                previews.update(entries)
                gridOrder.update(entries)
                let snoozes = snoozed.snoozes.count
                snoozed.endWoken(entries)
                if snoozed.snoozes.count != snoozes, persistsState { snoozed.save(to: SnoozedAgents.file) }
            }
            .onChange(of: gridOrder) { _, order in if persistsState { order.save(to: GridOrder.file) } }
            .onChange(of: layout) { _, layout in
                inboxQuery = ""
                inboxFilter = .all
                inboxSearching = false
                showsSnoozed = false
                if persistsState { DevicePrefs.update(in: DevicePrefs.file) { $0.agentsLayout = layout } }
            }
            .onChange(of: path, initial: true) { _, path in viewing(path.last) }
            .onChange(of: path) { old, new in
                // A swipe replaces the route, so it keeps the order the session opened with.
                if old.isEmpty, let route = new.last { snapshotOrder(opening: route) }
                navigationChanged()
            }
            .onChange(of: reselected) { showsUsage = false }
            .onChange(of: opening, initial: true) { _, route in
                guard let route else { return }
                snapshotOrder(opening: route)
                path = [route]
                opening = nil
            }
        }
        .followNotice(follows)
    }

    @ViewBuilder
    private var agents: some View {
        switch layout {
        case .grid:
            AgentGrid(
                entries: model.entries, previews: previews, order: $gridOrder, notice: model.closeNotice,
                approvalsCount: approvalsCount, reconnect: reconnect, showsLink: !tailnetStarting, follows: follows,
                closePane: { model.beginClose(.pane, route: $0) }, margin: listMargin, menu: menu, dragging: $dragging
            )
            .refreshable { await refresh() }
            .task(id: core != nil && scenePhase == .active && !newTask && !showsUsage) {
                guard let core, scenePhase == .active, !newTask, !showsUsage else { return }
                previews.showChat(DevicePrefs.load(from: DevicePrefs.file).defaultSessionView == .chat)
                await previews.run(core: core)
            }
        case .inbox:
            AgentInbox(
                entries: model.entries, notice: model.closeNotice, follows: follows,
                seen: seen, markUnseen: markUnseen, snoozed: snoozed, snooze: snooze,
                query: $inboxQuery, searching: $inboxSearching, filter: $inboxFilter, showsSnoozed: $showsSnoozed, margin: listMargin,
                refresh: refresh, menu: menu
            )
        case .list:
            TimelineView(.periodic(from: .now, by: 60)) { context in
                List {
                    if let notice = model.closeNotice {
                        Label(notice, systemImage: "exclamationmark.triangle")
                            .font(.footnote)
                            .foregroundStyle(.warning)
                    }
                    if model.entries.isEmpty {
                        NoMachines()
                    }
                    ForEach(model.entries) { entry in
                        Section {
                            if entry.flock?.details != nil && entry.agents.isEmpty {
                                Text("No agents running").foregroundStyle(.secondary)
                            }
                            ForEach(entry.agents, id: \.terminalId) { agent in
                                let route = AgentRoute(machineId: entry.id, terminalId: agent.terminalId)
                                NavigationLink(value: route) {
                                    AgentRow(
                                        agent: agent, workspace: entry.workspaceLabel(for: agent),
                                        followed: follows?.isFollowing(route) == true, now: context.date
                                    )
                                }
                                .contextMenu { menu(agent, route) }
                            }
                            .opacity(entry.linkDown ? 0.5 : 1)
                            if !entry.terminals.isEmpty {
                                Text("Terminals").font(.footnote.weight(.semibold)).foregroundStyle(.secondary)
                                ForEach(entry.terminals, id: \.terminalId) { terminal in
                                    let route = AgentRoute(machineId: entry.id, terminalId: terminal.terminalId)
                                    NavigationLink(value: route) {
                                        TerminalRow(terminal: terminal)
                                    }
                                    .contextMenu {
                                        Button("Close pane", systemImage: "xmark.square", role: .destructive) {
                                            model.beginClose(.pane, route: route)
                                        }
                                        .tint(.red)
                                        Button("Archive", systemImage: "archivebox", role: .destructive) {
                                            model.beginClose(.archive, route: route)
                                        }
                                        .tint(.red)
                                    }
                                }
                                .opacity(entry.linkDown ? 0.5 : 1)
                            }
                        } header: {
                            Button {
                                reconnect(entry)
                            } label: {
                                MachineHeader(entry: entry, approvalsCount: approvalsCount(entry), showsLink: !tailnetStarting, now: context.date)
                                    .font(.footnote.weight(.semibold))
                                    .textCase(nil)
                            }
                            .buttonStyle(.plain)
                        }
                    }
                }
                .listSectionSpacing(.compact)
                .modifier(PagerListMargin(edge: .trailing, margin: listMargin))
            }
            .refreshable { await refresh() }
        }
    }

    /// The agents in the order the current layout shows them; `filtered` applies the inbox's filter, search and collapsed snoozed section.
    private func layoutOrder(_ entries: [MachineFlockEntry], starred: Set<AgentRoute>, filtered: Bool) -> [AgentRoute] {
        switch layout {
        case .grid:
            return entries.flatMap { entry in
                let agents = entry.gridAgents(starred: starred, order: gridOrder)
                return (agents.starred + agents.rest).map { AgentRoute(machineId: entry.id, terminalId: $0.terminalId) }
            }
        case .inbox:
            let items = InboxItem.items(in: entries).filter { !filtered || $0.matches(inboxQuery) }
            let rows = InboxItem.rows(items, filter: filtered && inboxQuery.isEmpty ? inboxFilter : .all, snoozed: snoozed, now: .now)
            let asleep = filtered && !showsSnoozed ? [] : rows.asleep.map(\.0)
            return (InboxSection.allCases.flatMap { rows.active[$0] ?? [] } + asleep).map(\.route)
        case .list:
            return FlockOrder.routes(in: entries)
        }
    }

    /// A notification, a Live Activity or New Task can open an agent the layout does not show; that session follows the layout as it is now.
    private func snapshotOrder(opening route: AgentRoute) {
        let shown = layoutOrder(model.entries, starred: previews.starred, filtered: true)
        swipeOrder = shown.contains(route) ? shown : nil
    }

    // Each page takes its own refreshable: on the pager it reached every scroll view, the inbox's filter row too.
    private func refresh() async {
        for machine in machines { try? core?.reconnect(machineId: machine.id) }
        await model.refresh(core: core, machines: machines)
        usageRefresh += 1
    }

    private func approvalsCount(_ entry: MachineFlockEntry) -> Int {
        approvals.map { $0.items.filter { $0.machine.id == entry.id }.count } ?? Int(entry.flock?.approvalsCount ?? 0)
    }

    private func reconnect(_ entry: MachineFlockEntry) {
        try? core?.reconnect(machineId: entry.id)
    }

    private func markUnseen(_ agent: AgentSummary, route: AgentRoute) {
        seen.markUnseen(agent, route: route)
        if persistsState { seen.save(to: SeenAgents.file) }
    }

    private func snooze(_ agent: AgentSummary, route: AgentRoute, until wake: Date?) {
        if let wake {
            snoozed.snooze(agent, route: route, until: wake)
            snoozed.endWoken(model.entries)
        } else {
            snoozed.wakeNow(route)
        }
        if persistsState { snoozed.save(to: SnoozedAgents.file) }
    }

    @ViewBuilder
    private func menu(_ agent: AgentSummary, _ route: AgentRoute) -> some View {
        if let follows {
            FollowMenuItem(follows: follows, route: route).tint(.primary)
            Divider()
        }
        Button("Close pane", systemImage: "xmark.square", role: .destructive) {
            model.beginClose(.pane, route: route)
        }
        .tint(.red)
        Button("Close workspace", systemImage: "xmark.rectangle.portrait", role: .destructive) {
            model.beginClose(.workspace(id: agent.workspaceId), route: route)
        }
        .tint(.red)
        Button("Archive", systemImage: "archivebox", role: .destructive) {
            model.beginClose(.archive, route: route)
        }
        .tint(.red)
    }
}

/// A List on a pager page that is off screen when SwiftUI lays it out gets the margin of where that page sits: Usage, to the
/// right, loses its leading margin, and the Agents lists, to the left after a rotation on Usage, their trailing one. This
/// side takes the screen's minimum margin plus the List's own safe area instead.
struct PagerListMargin: ViewModifier {
    let edge: HorizontalEdge
    let margin: CGFloat?

    func body(content: Content) -> some View {
        GeometryReader { geometry in
            let safeArea = edge == .leading ? geometry.safeAreaInsets.leading : geometry.safeAreaInsets.trailing
            content.contentMargins(edge == .leading ? .leading : .trailing, margin.map { $0 + safeArea }, for: .scrollContent)
        }
    }
}

struct ViewSwitch: View {
    @Binding var showsUsage: Bool

    var body: some View {
        GlassPicker(title: "View", selection: $showsUsage, options: [("Agents", false), ("Usage", true)], toolbar: true)
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
    let now: Date

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            // With larger text the pills drop under the name rather than break it mid-word.
            ViewThatFits(in: .horizontal) {
                HStack {
                    MachineName(machine: entry.machine)
                    Spacer()
                    status
                }
                VStack(alignment: .leading, spacing: 4) {
                    MachineName(machine: entry.machine)
                    HStack { status }
                }
            }
            if showsLink, entry.flock?.versionMismatch == true, let message = entry.flock?.lastError {
                Text(message)
                    .font(.footnote)
                    .foregroundStyle(.warning)
            } else if showsLink, entry.flock?.link != .connecting, let reason = entry.flock?.reason {
                Text(reason.label(machine: entry.machine.label, now: now))
                    .font(.footnote)
                    .foregroundStyle(.secondary)
            }
        }
    }

    @ViewBuilder
    private var status: some View {
        if approvalsCount > 0 {
            StatusPill(state: .blocked, title: "\(approvalsCount) approval\(approvalsCount == 1 ? "" : "s")")
        }
        if showsLink, let link = entry.flock?.link {
            if link == .connecting {
                ProgressView().controlSize(.mini)
                Text(entry.flock?.details != nil ? "reconnecting" : "connecting").foregroundStyle(.secondary)
            } else if link != .connected {
                StatusPill(state: .idle, title: link.label)
            }
        }
    }
}

private struct AgentRow: View {
    let agent: AgentSummary
    let workspace: String?
    let followed: Bool
    let now: Date
    @Environment(\.dynamicTypeSize) private var typeSize
    @ScaledMetric(relativeTo: .subheadline) private var lift: CGFloat = 6

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 12) {
            StatusIcon(state: agent.status)
                .accessibilityHidden(true)
                // Centered on the title's first line rather than sitting on its baseline.
                .alignmentGuide(.firstTextBaseline) { $0[VerticalAlignment.center] + lift }
            VStack(alignment: .leading, spacing: 2) {
                Text(agent.displayTitle).font(.subheadline.weight(.semibold)).lineLimit(1)
                HStack(spacing: 4) {
                    if let kind = agent.kind {
                        AgentKindLabel(kind: kind, iconOnly: true)
                    }
                    if followed {
                        Image(systemName: "pin.fill")
                            .foregroundStyle(.tint)
                            .accessibilityLabel("Followed")
                    }
                    if let workspace {
                        Text(workspace)
                        Text(verbatim: "·").fixedSize()
                    }
                    StatusAge(agent: agent, now: now).fixedSize()
                }
                .lineLimit(1)
                .font(.caption)
                .foregroundStyle(.metadata)
                // At accessibility sizes a trailing column cut the title and the workspace to a few letters.
                if typeSize.isAccessibilitySize, let left = agent.contextLeft { ContextRing(left: left, font: .caption2) }
            }
            Spacer()
            if !typeSize.isAccessibilitySize, let left = agent.contextLeft { ContextRing(left: left, font: .caption2, aligned: true) }
        }
    }
}

private struct TerminalRow: View {
    let terminal: TerminalSummary

    var body: some View {
        HStack(spacing: 12) {
            AgentKindLabel(kind: "terminal", iconOnly: true).frame(minWidth: 20)
            VStack(alignment: .leading, spacing: 2) {
                Text(terminal.displayTitle).lineLimit(1)
                if let workspace = terminal.workspaceLabel {
                    Text(workspace).font(.caption).foregroundStyle(.metadata).lineLimit(1)
                }
            }
            Spacer()
            Image(systemName: terminal.locked ? "lock.fill" : "lock.open")
                .font(.caption)
                .foregroundStyle(.secondary)
                .accessibilityLabel(terminal.locked ? "Locked" : "Unlocked")
        }
    }
}

/// The agent kind as herdr names it, with its own icon for the kinds New Task starts.
/// "terminal" is a plain shell pane.
struct AgentKindLabel: View {
    let kind: String
    var iconOnly = false
    // The logos are bitmaps of a fixed size; alone beside text they grow with it, as a symbol would.
    @ScaledMetric(relativeTo: .caption) private var scale: CGFloat = 1

    var body: some View {
        switch (logo, iconOnly) {
        case let ((title, image)?, true):
            let size = UIImage(named: image)?.size ?? .zero
            Image(image)
                .resizable()
                .scaledToFit()
                .frame(width: size.width * scale, height: size.height * scale)
                .accessibilityLabel(title)
        case let ((title, image)?, false): Label(title, image: image).labelStyle(KindLabelStyle())
        case (nil, true): Label(kind, systemImage: "terminal").labelStyle(.iconOnly)
        case (nil, false): Label(kind, systemImage: "terminal").labelStyle(KindLabelStyle())
        }
    }

    private var logo: (String, String)? {
        switch kind {
        case "claude": ("Claude", "Claude")
        case "codex": ("Codex", "Codex")
        case "copilot": ("Copilot", "Copilot")
        case "terminal": ("Terminal", "Ghostty")
        default: nil
        }
    }
}

struct KindLabelStyle: LabelStyle {
    func makeBody(configuration: Configuration) -> some View {
        HStack(spacing: 4) {
            configuration.icon
            configuration.title
        }
    }
}

struct StatusPill: View {
    let state: AgentState
    let title: String
    @Environment(\.colorScheme) private var colorScheme

    var body: some View {
        Label(title, systemImage: StatusIcon.symbol(state))
            .labelStyle(KindLabelStyle())
            .font(.caption.weight(.semibold))
            .foregroundStyle(state.color)
            .padding(.horizontal, 8)
            .padding(.vertical, 4)
            .padding(2)
            .glassEffect(.regular, in: .capsule)
            // Dark glass over a lighter surface such as the terminal turns gray; the dim keeps the label at 4.5:1 or more.
            .background(colorScheme == .dark ? .black.opacity(0.75) : .clear, in: .capsule)
            .fixedSize()
    }
}

/// How long the agent has been in its status, compact, read in full by VoiceOver.
struct StatusAge: View {
    let agent: AgentSummary
    let now: Date

    var body: some View {
        Text(Elapsed.compact(sinceMs: agent.statusSinceMs, now: now))
            .monospacedDigit()
            .accessibilityLabel(Elapsed.spoken(agent.status, sinceMs: agent.statusSinceMs, now: now))
    }
}

struct StatusIcon: View {
    let state: AgentState
    var size: CGFloat = 20
    var dimsIdle = true
    @ScaledMetric(relativeTo: .subheadline) private var scale: CGFloat = 1
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Environment(\.appAccent) private var appAccent

    var body: some View {
        // Grows with the text beside it, up to half again its size, so a glyph never outweighs the title it marks.
        let side = size * min(scale, 1.5)
        let unit = side / 20
        Group {
            switch state {
            // Core Animation spins it; a SwiftUI repeatForever redraws the app every frame (battery).
            case .working where reduceMotion:
                Circle()
                    .trim(from: 0, to: 0.7)
                    .stroke(appAccent, style: StrokeStyle(lineWidth: 2.5 * unit, lineCap: .round))
                    .padding(2 * unit)
            // The ring keeps a steady silhouette, like Done's circle, while the spinner's spokes fade in turn.
            case .working:
                ZStack {
                    Circle().stroke(appAccent, lineWidth: 1.5 * unit).padding(2.5 * unit)
                    ProgressView().controlSize(.small).tint(appAccent).scaleEffect(0.6 * unit)
                }
            // Idle's ring is the quietest, near the 3:1 floor for a glyph on the rows and cards; the session header, over
            // the terminal, keeps the full color.
            default: Image(systemName: Self.symbol(state)).font(.system(size: 16 * unit)).foregroundStyle(state.color.opacity(state == .idle && dimsIdle ? 0.75 : 1))
            }
        }
        .frame(width: side, height: side)
        .accessibilityElement()
        .accessibilityLabel(state.label)
    }

    /// One family of outline circles the size of the working ring; blocked is the one filled disc, so it stays the loudest,
    /// and idle's is dashed, so it stays the quietest rather than reading as an unchecked Done.
    static func symbol(_ state: AgentState) -> String {
        switch state {
        case .blocked: "exclamationmark.circle.fill"
        case .done: "checkmark.circle"
        case .idle: "circle.dashed"
        default: "questionmark.circle"
        }
    }
}

extension ShapeStyle where Self == Color {
    // Secondary gray is about 3.4:1 on white and 4.2:1 on the session bar over the terminal; both get a stronger gray, 4.5:1 or more.
    static var metadata: Color {
        Color(uiColor: UIColor {
            $0.userInterfaceStyle == .dark
                ? UIColor.secondaryLabel.resolvedColor(with: $0).withAlphaComponent(0.7)
                : UIColor(red: 0.42, green: 0.42, blue: 0.44, alpha: 1)
        })
    }
}

struct ContextRing: View {
    let left: UInt8
    var font: Font = .caption
    /// Takes the width "100%" would, so a column of rows keeps one width; the chip stays beside its figure.
    var aligned = false

    var body: some View {
        let left = min(left, 100)
        Group {
            if aligned {
                label("100%").hidden().overlay(alignment: .trailing) { label("\(left)%") }
            } else {
                label("\(left)%")
            }
        }
            .font(font.monospacedDigit())
            .foregroundStyle(left <= 20 ? Self.tone(left) : .metadata)
            .fixedSize()
            .accessibilityElement()
            .accessibilityLabel("\(left)% of context left")
    }

    private func label(_ figure: String) -> some View {
        Label(figure, systemImage: "memorychip").labelStyle(KindLabelStyle())
    }

    /// The one warning rule for context and usage limits: accent, orange from 80% used, red from 90%.
    static func tone(_ left: UInt8) -> Color {
        switch 100 - Int(min(left, 100)) {
        case ..<80: .accentColor
        case ..<90: .warning
        default: AgentState.blocked.color
        }
    }
}

extension AgentState {
    var label: String { AgentActivityStatus(self).label }

    var color: Color {
        switch self {
        case .working: .accentColor
        // System red is about 3.3:1 as small text on light glass, even its high-contrast variant 4.1:1; light mode gets a deeper red.
        // On dark glass it is about 3.2:1 and its high-contrast variant 3.8:1; dark mode gets a lighter red, 4.5:1 or more.
        case .blocked:
            Color(uiColor: UIColor {
                $0.userInterfaceStyle == .dark ? UIColor(red: 1, green: 0.54, blue: 0.5, alpha: 1) : UIColor(red: 0.78, green: 0, blue: 0.08, alpha: 1)
            })
        // System green is about 2.2:1 on white and system gray 4.3:1 on the dark status pill; both read 4.5:1 or more
        // as the pill's label on light and dark glass, and 3:1 or more as row glyphs.
        case .done:
            Color(uiColor: UIColor { $0.userInterfaceStyle == .dark ? .systemGreen.resolvedColor(with: $0) : UIColor(red: 0, green: 0.47, blue: 0.18, alpha: 1) })
        case .idle:
            Color(uiColor: UIColor {
                $0.userInterfaceStyle == .dark ? UIColor(red: 0.6, green: 0.6, blue: 0.62, alpha: 1) : UIColor(red: 0.42, green: 0.42, blue: 0.44, alpha: 1)
            })
        case .unknown: AgentActivityStatus(self).color
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

extension OfflineReason {
    func label(machine: String, now: Date) -> String {
        switch self {
        case .phoneOffline:
            "This phone is not connected to Tailscale. Sign in again or check the network."
        case .asleep(let lastSeenMs):
            "\(machine) is asleep or off." + (lastSeenMs.map { " Last seen \(Elapsed.spoken(sinceMs: $0, now: now)) ago." } ?? "")
        case .notAnswering:
            "\(machine) is online, but collied is not answering. Run collied start there."
        case .unreachable:
            "Cannot reach \(machine)."
        }
    }
}

extension String {
    var nilIfEmpty: String? { isEmpty ? nil : self }
}
