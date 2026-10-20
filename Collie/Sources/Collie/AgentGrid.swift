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
                        .foregroundStyle(.orange)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .padding(.horizontal)
                }
                if let moveError {
                    Text(moveError).font(.footnote).foregroundStyle(.orange).padding(.horizontal)
                }
                if entries.isEmpty {
                    NoMachines()
                }
                LazyVStack(alignment: .leading, spacing: 16) {
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
                                    alignment: .leading, spacing: 16
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
                                if entry.flock?.details != nil && entry.agents.isEmpty {
                                    Text("No agents running").foregroundStyle(.secondary)
                                }
                            }
                            .padding(.top, 8)
                        }
                    }
                }
                .onGeometryChange(for: CGFloat.self) { $0.size.width } action: { gridWidth = $0 }
                .padding(.horizontal)
                .padding(.bottom)
            }
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
            ChatPreview(messages: conversation, lines: starred ? 6 : 3)
        } else {
            TerminalPreview(ansi: screen ?? "")
        }
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            preview
                .frame(height: typeSize.isAccessibilitySize && agent.status != .blocked && !agent.subagents.isEmpty
                    ? subagentPreviewHeight * (starred ? 2 : 1)
                    : (starred ? 240 : 120))
                .accessibilityHidden(true)
                .overlay(alignment: .top) {
                    if agent.status != .blocked, !agent.subagents.isEmpty {
                        let limit = starred ? 4 : 2
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
                        .glassEffect(.clear, in: RoundedRectangle(cornerRadius: 10))
                        .background(SessionBarDim(), in: RoundedRectangle(cornerRadius: 10))
                        .environment(\.colorScheme, .dark)
                    }
                }
                .clipShape(RoundedRectangle(cornerRadius: 10))
                .allowsHitTesting(false)
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
                        if agent.status == .blocked, !agent.subagents.isEmpty {
                            Text(verbatim: "·")
                            Text(agent.subagents.count == 1 ? "1 subagent" : "\(agent.subagents.count) subagents").fixedSize()
                        }
                        if let pullRequest = agent.pullRequest {
                            PullRequestBadge(pullRequest: pullRequest, watch: agent.prWatch, compact: true)
                        }
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
        .accessibilityActions {
            if let url = agent.pullRequest.flatMap(PullRequestBadge.url) {
                Button("Open pull request") { openURL(url) }
            }
        }
    }
}

/// Read-only, like `TerminalPreview`: it follows the bottom of the conversation.
private struct ChatPreview: View {
    let messages: [HistoryMessage]
    let lines: Int

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            ForEach(messages, id: \.id) { message in
                let user = message.role == "user"
                Text((try? AttributedString(markdown: message.text, options: .init(interpretedSyntax: .inlineOnlyPreservingWhitespace))) ?? AttributedString(message.text))
                    .lineLimit(lines)
                    .padding(user ? 6 : 0)
                    .background {
                        if user {
                            RoundedRectangle(cornerRadius: 8).fill(.tint.opacity(0.1))
                        }
                    }
                    .padding(.leading, user ? 16 : 0)
                    .frame(maxWidth: .infinity, alignment: user ? .trailing : .leading)
            }
        }
        .font(.caption2)
        .fixedSize(horizontal: false, vertical: true)
        .padding(6)
        .frame(maxWidth: .infinity, minHeight: 0, maxHeight: .infinity, alignment: .bottom)
        .background(.fill.tertiary)
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
