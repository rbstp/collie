import CollieCore
import GhosttyTerminal
import SwiftUI

struct AgentGrid<Menu: View>: View {
    let entries: [MachineFlockEntry]
    let previews: PreviewModel
    @Binding var order: GridOrder
    let notice: String?
    let approvalsCount: (MachineFlockEntry) -> Int
    let reconnect: (MachineFlockEntry) -> Void
    let showsLink: Bool
    let follows: FollowModel?
    let closePane: (AgentRoute) -> Void
    let margin: CGFloat?
    @ViewBuilder let menu: (AgentSummary, AgentRoute) -> Menu
    @Environment(\.dynamicTypeSize) private var typeSize
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Binding var dragging: GridDrag?
    @State private var gridWidth: CGFloat = 0
    @State private var moving = false
    @State private var moveError: String?

    var body: some View {
        TimelineView(.periodic(from: .now, by: 60)) { context in
            ScrollView {
                if let notice {
                    Label(notice, systemImage: "exclamationmark.triangle")
                        .font(.footnote)
                        .foregroundStyle(.warning)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .padding(.horizontal)
                }
                if let moveError {
                    Text(moveError).font(.footnote).foregroundStyle(.warning).padding(.horizontal)
                }
                if entries.isEmpty {
                    NoMachines()
                }
                LazyVStack(alignment: .leading, spacing: 12) {
                    ForEach(entries) { entry in
                        Section {
                            let agents = entry.gridAgents(starred: previews.starred, order: order)
                            if dragging?.route.machineId == entry.id, dragging?.starred == false, agents.starred.isEmpty {
                                dropShelf(starred: true, in: entry)
                            }
                            ForEach(agents.starred, id: \.terminalId) { card($0, in: entry, starred: true, now: context.date) }
                            if !agents.rest.isEmpty {
                                LazyVGrid(
                                    columns: typeSize.isAccessibilitySize ? [GridItem(.flexible())] : [GridItem(.adaptive(minimum: 150), spacing: 12)],
                                    alignment: .leading, spacing: 12
                                ) {
                                    let columns = typeSize.isAccessibilitySize ? 1 : max(1, Int((gridWidth + 12) / (150 + 12)))
                                    let axes: Axis.Set = columns == 1 ? .vertical : agents.rest.count <= columns ? .horizontal : [.horizontal, .vertical]
                                    ForEach(agents.rest, id: \.terminalId) { card($0, in: entry, starred: false, now: context.date, axes: axes) }
                                }
                            }
                            if dragging?.route.machineId == entry.id, dragging?.starred == true, agents.rest.isEmpty {
                                dropShelf(starred: false, in: entry)
                            }
                            if !entry.terminals.isEmpty {
                                Text("Terminals").font(.footnote.weight(.semibold)).foregroundStyle(.secondary).padding(.horizontal, 20)
                                LazyVGrid(
                                    columns: typeSize.isAccessibilitySize ? [GridItem(.flexible())] : [GridItem(.adaptive(minimum: 150), spacing: 12)],
                                    alignment: .leading, spacing: 12
                                ) {
                                    ForEach(entry.terminals, id: \.terminalId) { terminalCard($0, in: entry) }
                                }
                            }
                        } header: {
                            VStack(alignment: .leading, spacing: 6) {
                                let header = MachineHeader(
                                    entry: entry, showsName: entries.count > 1, approvalsCount: approvalsCount(entry), showsLink: showsLink, now: context.date
                                )
                                if !header.isEmpty {
                                    let label = header.font(.footnote.weight(.semibold)).foregroundStyle(.secondary)
                                    // Reconnecting a connected machine shows nothing: alone, the approvals pill is plain status.
                                    if entries.count > 1 || entry.flock?.link != .connected {
                                        Button {
                                            reconnect(entry)
                                        } label: {
                                            label
                                        }
                                        .buttonStyle(.plain)
                                    } else {
                                        label
                                    }
                                }
                                if entry.flock?.details != nil && entry.agents.isEmpty {
                                    Text("No agents running").foregroundStyle(.secondary)
                                }
                            }
                            // The inset of a List section header, so headers line up across layouts and Usage.
                            .padding(.horizontal, 20)
                            .padding(.top, 8)
                        }
                    }
                }
                .onGeometryChange(for: CGFloat.self) { $0.size.width } action: { gridWidth = $0 }
                .padding(.horizontal, margin ?? 16)
                .padding(.bottom)
            }
            .background(Color(.systemGroupedBackground))
        }
    }

    private func card(_ agent: AgentSummary, in entry: MachineFlockEntry, starred: Bool, now: Date, axes: Axis.Set = .vertical) -> some View {
        let route = AgentRoute(machineId: entry.id, terminalId: agent.terminalId)
        return NavigationLink(value: route) {
            AgentCard(
                agent: agent, screen: previews.screens[route], conversation: previews.conversation(route, status: agent.status),
                workspace: entry.workspaceLabel(for: agent),
                followed: follows?.isFollowing(route) == true, starred: starred, now: now
            )
        }
        .buttonStyle(.plain)
        .opacity(entry.linkDown ? 0.5 : 1)
        .contextMenu {
            Button(starred ? "Unstar" : "Star", systemImage: starred ? "star.slash" : "star") { Task { await previews.toggleStar(route) } }
                .tint(.primary)
                .disabled(entry.flock?.link != .connected)
            menu(agent, route)
        }
        .modifier(GridDragSource(route: route, starred: starred, dragging: $dragging))
        .modifier(GridDropTarget(dragging: $dragging, machineId: entry.id, destination: route, axes: axes) { source, after in
            move(source, in: entry, starred: starred, destination: route, after: after)
        })
        .onAppear { previews.appeared(route) }
        .onDisappear { previews.disappeared(route) }
    }

    private func dropShelf(starred: Bool, in entry: MachineFlockEntry) -> some View {
        Label(starred ? "Drop here to star" : "Drop here to unstar", systemImage: starred ? "star" : "star.slash")
            .font(.caption)
            .foregroundStyle(.secondary)
            .frame(maxWidth: .infinity, minHeight: 36)
            .glassEffect(.regular, in: RoundedRectangle(cornerRadius: 12))
            .contentShape(Rectangle())
            .modifier(GridDropTarget(dragging: $dragging, machineId: entry.id) { source, _ in
                move(source, in: entry, starred: starred, after: !starred)
            })
    }

    private func move(_ source: AgentRoute, in entry: MachineFlockEntry, starred: Bool, destination: AgentRoute? = nil, after: Bool = false) -> Bool {
        guard !moving, source.machineId == entry.id, source != destination,
              entry.agents.contains(where: { $0.terminalId == source.terminalId }) else { return false }
        moving = true
        moveError = nil
        Task {
            defer { moving = false }
            guard await previews.setStar(source, starred: starred) else {
                moveError = "Could not move agent. Reconnect and try again."
                return
            }
            withAnimation(reduceMotion ? nil : .default) {
                _ = order.move(source, to: destination, in: entry, starred: previews.starred, after: after)
            }
        }
        return true
    }
}

extension AgentGrid {
    /// No live preview: reading a shell needs a grant.
    private func terminalCard(_ terminal: TerminalSummary, in entry: MachineFlockEntry) -> some View {
        let route = AgentRoute(machineId: entry.id, terminalId: terminal.terminalId)
        return NavigationLink(value: route) {
            HStack(spacing: 8) {
                AgentKindLabel(kind: "terminal", iconOnly: true)
                VStack(alignment: .leading, spacing: 1) {
                    Text(terminal.displayTitle).font(.footnote.weight(.semibold)).lineLimit(1)
                    if let workspace = terminal.workspaceLabel, workspace != terminal.displayTitle {
                        Text(workspace)
                            .font(.caption)
                            .foregroundStyle(.metadata)
                            .lineLimit(1)
                    }
                }
                Spacer(minLength: 0)
                Image(systemName: terminal.locked ? "lock.fill" : "lock.open")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .accessibilityLabel(terminal.locked ? "Locked" : "Unlocked")
            }
            .padding(.horizontal, 14)
            .padding(.vertical, 11)
            .frame(maxHeight: .infinity)
            .background(Color(.secondarySystemGroupedBackground), in: RoundedRectangle(cornerRadius: 18))
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .opacity(entry.linkDown ? 0.5 : 1)
        .contextMenu {
            Button("Close pane", systemImage: "xmark.square", role: .destructive) { closePane(route) }
                .tint(.red)
        }
    }
}

extension MachineFlockEntry {
    func gridAgents(starred: Set<AgentRoute>, order: GridOrder = GridOrder()) -> (starred: [AgentSummary], rest: [AgentSummary]) {
        let isStarred = { (agent: AgentSummary) in starred.contains(AgentRoute(machineId: id, terminalId: agent.terminalId)) }
        let ordered = order.agents(in: self)
        return (ordered.filter(isStarred), ordered.filter { !isStarred($0) })
    }
}

struct AgentCard: View {
    let agent: AgentSummary
    let screen: String?
    let conversation: [HistoryMessage]?
    let workspace: String?
    let followed: Bool
    let starred: Bool
    let now: Date
    @ScaledMetric(relativeTo: .caption2) private var subagentPreviewHeight: CGFloat = 120
    @Environment(\.dynamicTypeSize) private var typeSize
    @Environment(\.openURL) private var openURL

    @ViewBuilder private var preview: some View {
        if let conversation {
            ChatPreview(messages: conversation.suffix(starred ? 4 : 2), lines: starred ? 2 : 3)
        } else {
            TerminalPreview(ansi: screen ?? "", fontSize: starred ? 11 : 10)
                .overlay(alignment: .top) {
                    // Hides the row cut at the top: solid first, then a fade.
                    let background = Color(uiColor: TerminalPreview.panel)
                    LinearGradient(stops: [.init(color: background, location: 0), .init(color: background, location: 0.4), .init(color: .clear, location: 1)],
                                   startPoint: .top, endPoint: .bottom)
                        .frame(height: 12)
                }
        }
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            preview
                .frame(height: typeSize.isAccessibilitySize && agent.status != .blocked && !agent.subagents.isEmpty
                    ? subagentPreviewHeight * (starred ? 2 : 1)
                    // Four one-line messages, the most a chat preview holds, fill about 110 pt; 150 left a blank band under them.
                    : (starred ? (conversation != nil ? 110 : 150) : 84))
                .accessibilityHidden(true)
                .overlay(alignment: .top) {
                    if agent.status != .blocked, !agent.subagents.isEmpty {
                        let limit = starred ? 4 : 1
                        VStack(alignment: .leading, spacing: 2) {
                            ForEach(Array(agent.subagents.prefix(limit).enumerated()), id: \.offset) { _, subagent in
                                HStack(spacing: 4) {
                                    Text(verbatim: subagent.title).lineLimit(1)
                                    Spacer(minLength: 0)
                                    Text(verbatim: subagent.elapsed(now: now)).monospacedDigit().fixedSize()
                                }
                            }
                            if agent.subagents.count > limit {
                                Text("+\(agent.subagents.count - limit) more")
                            }
                        }
                        .font(.caption2)
                        .foregroundStyle(.white)
                        .padding(6)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .glassEffect(.clear, in: RoundedRectangle(cornerRadius: 12))
                        .background(SessionBarDim(), in: RoundedRectangle(cornerRadius: 12))
                        .environment(\.colorScheme, .dark)
                    }
                }
                .clipShape(RoundedRectangle(cornerRadius: 12))
                .allowsHitTesting(false)
            VStack(alignment: .leading, spacing: 2) {
                HStack(spacing: 6) {
                    StatusIcon(state: agent.status, size: 16)
                        .accessibilityHidden(true)
                    Text(agent.displayTitle)
                        .font(.footnote.weight(.semibold))
                        .lineLimit(1)
                        .frame(maxWidth: .infinity, alignment: .leading)
                    if let limit = agent.planLimit {
                        LimitBadge(limit: limit, compact: true)
                    }
                    if let pullRequest = agent.pullRequest {
                        PullRequestBadge(pullRequest: pullRequest, watch: agent.prWatch, compact: true)
                    }
                    if starred {
                        Image(systemName: "star.fill")
                            .font(.caption2)
                            .foregroundStyle(.yellow)
                            .accessibilityLabel("Starred")
                    }
                }
                HStack(spacing: 6) {
                    // Under the status icon, so the metadata lines up with the title.
                    Group {
                        if let kind = agent.kind {
                            AgentKindLabel(kind: kind, iconOnly: true)
                        }
                    }
                    .frame(width: typeSize.isAccessibilitySize ? nil : 16)
                    HStack(spacing: 3) {
                        if followed {
                            Image(systemName: "pin.fill")
                                .foregroundStyle(.tint)
                                .accessibilityLabel("Followed")
                        }
                        if let workspace {
                            Text(workspace).layoutPriority(1)
                            Text(verbatim: "·").fixedSize()
                        }
                        StatusAge(agent: agent, now: now).fixedSize()
                        if agent.status == .blocked, !agent.subagents.isEmpty {
                            Text(verbatim: "·").fixedSize()
                            Text(agent.subagents.count == 1 ? "1 subagent" : "\(agent.subagents.count) subagents")
                        }
                    }
                    .font(.caption2)
                    .foregroundStyle(.metadata)
                    .lineLimit(1)
                    Spacer(minLength: 0)
                    if let left = agent.contextLeft, ContextRing.shows(left) {
                        ContextRing(left: left, font: .caption2)
                    }
                }
            }
            .padding(.horizontal, 6)
            .padding(.top, 8)
            .padding(.bottom, 4)
        }
        // Cards in a row share its height; any extra goes below the text, never between preview and title.
        .frame(maxHeight: .infinity, alignment: .top)
        .padding(6)
        .background(Color(.secondarySystemGroupedBackground), in: RoundedRectangle(cornerRadius: 18))
        .contentShape(Rectangle())
        .accessibilityElement(children: .combine)
        .accessibilityActions {
            if let url = agent.pullRequest.flatMap(PullRequestBadge.url) {
                Button("Open pull request") { openURL(url) }
            }
        }
    }
}

/// Read-only, like `TerminalPreview`: it follows the bottom of the conversation.
struct ChatPreview: View {
    let messages: ArraySlice<HistoryMessage>
    let lines: Int

    var body: some View {
        let content = VStack(alignment: .leading, spacing: 4) {
            ForEach(messages, id: \.id) { message in
                let user = message.role == "user"
                Group {
                    if let line = message.origin?.wakeLine {
                        Text("\(Image(systemName: "eye")) \(line)").lineLimit(1)
                    } else {
                        let text = Self.compact(message.text)
                        Text((try? AttributedString(markdown: text, options: .init(interpretedSyntax: .inlineOnlyPreservingWhitespace))) ?? AttributedString(text))
                            .lineLimit(user ? 1 : lines)
                    }
                }
                .foregroundStyle(user ? AnyShapeStyle(.metadata) : AnyShapeStyle(.primary))
                .padding(user ? 6 : 0)
                .background {
                    if user {
                        // The card's own fill, on which `.metadata` reads at 4.5:1 or more; on the preview's gray it did not.
                        RoundedRectangle(cornerRadius: 12).fill(Color(.secondarySystemGroupedBackground))
                    }
                }
                .padding(.leading, user ? 16 : 0)
                .frame(maxWidth: .infinity, alignment: user ? .trailing : .leading)
            }
        }
        .font(.caption2)
        .fixedSize(horizontal: false, vertical: true)
        // Vertically 6: the 84-point preview fits a bubble and three lines.
        .padding(.horizontal, 10)
        .padding(.vertical, 6)
        ViewThatFits(in: .vertical) {
            content.frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
            content
                .frame(maxWidth: .infinity, minHeight: 0, maxHeight: .infinity, alignment: .bottom)
                // Fades out the line cut at the top.
                .mask(LinearGradient(stops: [.init(color: .clear, location: 0), .init(color: .black, location: 0.15)], startPoint: .top, endPoint: .bottom))
        }
        .background(.fill.tertiary)
    }

    /// Blank lines and code fences would spend the preview's few lines on nothing.
    nonisolated static func compact(_ text: String) -> String {
        text.split(separator: "\n").filter { !$0.allSatisfy(\.isWhitespace) && !$0.contains(/^\s*```\S*\s*$/) }.joined(separator: "\n")
    }
}

/// Read-only: no scrolling, selection or gestures; it follows the bottom of the snapshot.
private struct TerminalPreview: UIViewRepresentable {
    let ansi: String
    let fontSize: CGFloat

    /// The chat preview's tertiary fill over the card, opaque: the light terminal's own background vanished on a white card.
    static let panel = UIColor { traits in
        var fill: (r: CGFloat, g: CGFloat, b: CGFloat, a: CGFloat) = (0, 0, 0, 0)
        var card: (r: CGFloat, g: CGFloat, b: CGFloat, a: CGFloat) = (0, 0, 0, 0)
        UIColor.tertiarySystemFill.resolvedColor(with: traits).getRed(&fill.r, green: &fill.g, blue: &fill.b, alpha: &fill.a)
        UIColor.secondarySystemGroupedBackground.resolvedColor(with: traits).getRed(&card.r, green: &card.g, blue: &card.b, alpha: &card.a)
        return UIColor(red: card.r + (fill.r - card.r) * fill.a, green: card.g + (fill.g - card.g) * fill.a,
                       blue: card.b + (fill.b - card.b) * fill.a, alpha: 1)
    }

    func makeUIView(context: Context) -> GhosttyTerminalUIView {
        let view = GhosttyTerminalUIView(fontSize: fontSize)
        view.themeBackground = Self.panel
        view.wraps = true
        view.isUserInteractionEnabled = false
        view.showsVerticalScrollIndicator = false
        view.showsHorizontalScrollIndicator = false
        return view
    }

    func updateUIView(_ view: GhosttyTerminalUIView, context: Context) {
        view.fontSize = fontSize
        if !ansi.isEmpty {
            view.show(ansiSnapshot: ansi)
        }
    }
}
