import CollieCore
import GhosttyTerminal
import SwiftUI
import UIKit

struct AgentScreen: View {
    @State private var model: AgentModel
    let approvals: ApprovalsModel?
    let follows: FollowModel?
    let machineLabel: String?
    let showsMachine: Bool
    let neighbor: ((Int) -> AgentRoute?)?
    let switchAgent: (AgentRoute) -> Void
    let opened: (AgentSummary) -> Void
    let archived: (String) -> Void
    @State private var recordedOpen = false
    @State private var showingChanges = false
    @State private var showingChat = DevicePrefs.load(from: DevicePrefs.file).defaultSessionView == .chat
    @State private var searchingChat = false
    @State private var keyboardShown = false
    @State private var typing = false
    @State private var height: CGFloat = 0
    @State private var barsRoom: CGFloat = 0
    @State private var headerHeight: CGFloat = 0
    @State private var composerHeight: CGFloat = 0
    @State private var obscuredTop: CGFloat = 0
    @State private var obscuredBottom: CGFloat = 0
    @Environment(\.dismiss) private var dismiss
    @Environment(\.scenePhase) private var scenePhase
    @Environment(\.verticalSizeClass) private var verticalSizeClass
    @Environment(\.dynamicTypeSize) private var typeSize
    @Environment(\.neutralAccent) private var neutralAccent

    init(
        core: any AgentCore, route: AgentRoute, persistsState: Bool = true, approvals: ApprovalsModel?, follows: FollowModel?,
        machineLabel: String?, showsMachine: Bool, neighbor: ((Int) -> AgentRoute?)?,
        switchAgent: @escaping (AgentRoute) -> Void, opened: @escaping (AgentSummary) -> Void,
        archived: @escaping (String) -> Void
    ) {
        _model = State(initialValue: AgentModel(core: core, route: route, prefsFile: persistsState ? DevicePrefs.file : nil, draftsFile: persistsState ? AgentDrafts.file : nil, viewedFile: persistsState ? GitViewed.file : nil, machineLabel: machineLabel, resends: persistsState ? .shared : PromptResends()))
        self.approvals = approvals
        self.follows = follows
        self.machineLabel = machineLabel
        self.showsMachine = showsMachine
        self.neighbor = neighbor
        self.switchAgent = switchAgent
        self.opened = opened
        self.archived = archived
    }

    private var approvalItems: [ApprovalItem] {
        approvals?.items(machineId: model.route.machineId, terminalId: model.route.terminalId) ?? []
    }

    private var reviewing: Binding<ApprovalItem?> {
        Binding {
            approvalItems.first { approvals?.expanded.contains($0.id) == true }
        } set: { item in
            if item == nil, let shown = reviewing.wrappedValue { approvals?.toggleExpanded(shown) }
        }
    }

    private var blocked: BlockedInput? {
        approvals?.blockedInput(machineId: model.route.machineId, terminalId: model.route.terminalId)
    }

    private var noticeButtons: some View {
        ForEach(model.noticeOptions, id: \.digit) { option in
            Button(option.label) { Task { await model.answerNotice(option) } }
        }
    }

    private var title: String { model.agent?.displayTitle ?? model.terminal?.displayTitle ?? "Agent" }

    private var archiveButton: some View {
        Button("Archive", systemImage: "archivebox", role: .destructive) {
            model.close.begin(.archive)
        }
        .tint(.red)
        .disabled(model.archiving)
    }

    private var typingInLandscape: Bool { keyboardShown && verticalSizeClass == .compact }

    var body: some View {
        VStack(spacing: 0) {
            if let notice = model.paneNotice {
                PaneCard(model: model, notice: notice)
            }
            Group {
                if showingChat && !model.isTerminal {
                    ConversationView(model: model.conversation, searching: $searchingChat, load: model.history) { showingChat = false }
                        .environment(model)
                } else {
                    AgentTerminal(
                        ansi: model.screen, wraps: model.wrapLines, fontSize: model.fontSize, gestures: model.gestures,
                        perform: perform, resized: { model.fontSize = $0 }, resizeEnded: model.saveFontSize,
                        // The quote waits in the chat draft: typed into the terminal, its newlines would submit.
                        quote: model.isTerminal ? nil : {
                            model.quote($0)
                            showingChat = true
                        },
                        openFile: model.isTerminal ? nil : { model.fileLink = FileLink(path: $0) },
                        // Leaving the screen cancels an upload in flight and hides how a send ends; a draft and its uploaded files are kept.
                        neighbor: model.upload == nil && !model.sendingPrompt ? neighbor : nil,
                        switchAgent: switchAgent, obscuredTop: obscuredTop, obscuredBottom: obscuredBottom,
                        typing: $typing, type: { text in
                            model.type(text)
                            if !model.keepsKeyboard(after: text) { typing = false }
                        }, deleteBackward: { model.tap(.backspace) },
                        inputAgent: model.agent?.kind, tapInput: { model.placeCursor(at: $0, of: $1) }
                    ) { await model.refresh() }
                        // The keyboard too: the measured bottom inset already counts it, so a frame that stopped above it would count it twice.
                        .ignoresSafeArea(edges: [.top, .bottom])
                        .onGeometryChange(for: EdgeInsets.self) { $0.safeAreaInsets } action: {
                            obscuredTop = $0.top
                            obscuredBottom = $0.bottom
                        }
                        .background { Color(uiColor: GhosttyTerminalUIView.backgroundColor).ignoresSafeArea() }
                        .overlay {
                            if model.ansi.isEmpty, !(model.isTerminal && model.terminalLocked) {
                                ProgressView("Waiting for output…")
                            }
                        }
                        .overlay(alignment: .bottom) {
                            GlassEffectContainer(spacing: 8) {
                                VStack(spacing: 8) {
                                    if model.jumpBanner {
                                        Button("Jump to bottom", systemImage: "arrow.down") {
                                            Task { await model.jumpToBottom() }
                                        }
                                        .buttonStyle(.glassProminent)
                                        .onAccentFill()
                                        .padding(.bottom, 8)
                                    }
                                    if !model.noticeOptions.isEmpty {
                                        ViewThatFits(in: .horizontal) {
                                            HStack { noticeButtons }
                                            VStack { noticeButtons }
                                        }
                                        .buttonStyle(.glassProminent)
                                        .onAccentFill()
                                        .padding([.horizontal, .bottom], 8)
                                        .accessibilityElement(children: .contain)
                                        .accessibilityLabel("Claude Code notice")
                                    }
                                }
                            }
                        }
                }
            }
            .scrollEdgeEffectHidden(true, for: .top)
        }
        .safeAreaInset(edge: .top, spacing: 0) {
            let items = approvalItems
            let notice = approvals?.notice(machineId: model.route.machineId, terminalId: model.route.terminalId)
            if !typingInLandscape || !items.isEmpty || notice != nil {
                VStack(spacing: 0) {
                    if !typingInLandscape {
                        VStack(spacing: 0) {
                            AgentHeader(
                                model: model, title: title, approving: !items.isEmpty, machineLabel: showsMachine ? machineLabel : nil,
                                showingChat: $showingChat
                            )
                            if let subagents = model.agent?.subagents, !subagents.isEmpty {
                                SubagentList(subagents: subagents)
                                    .padding(.horizontal)
                                    .padding(.bottom, 6)
                            }
                        }
                        .onGeometryChange(for: CGFloat.self) { $0.size.height } action: { headerHeight = $0 }
                    }
                    // The approval cards share the header's surface, so the content runs under them as under the header.
                    if let approvals {
                        // Together the cards take at most 45% of the screen, keyboard or not, so the composer stays in
                        // reach, and a quarter in landscape; never more than the header and the composer leave them.
                        // A card that does not fit its share shows as a band, with the review sheet for the rest.
                        let share = verticalSizeClass == .compact ? 0.25 : 0.45
                        let room = barsRoom - (typingInLandscape ? 0 : headerHeight) - composerHeight
                        let cap = height > 0 && barsRoom > 0 ? min(height * share, room) / CGFloat(max(items.count, 1)) : .infinity
                        Group {
                            ForEach(items) { item in
                                ApprovalCard(model: approvals, item: item, maxHeight: cap, sendAnswer: model.typeAnswer)
                            }
                            // The open sheet shows its own copy, and this one peeked above it cut through its letters.
                            if let notice, approvals.noticeItem.map({ approvals.expanded.contains($0.id) }) != true {
                                ApprovalNotice(notice: notice).padding([.horizontal, .bottom])
                            }
                        }
                    }
                }
                .background {
                    let page = showingChat && !model.isTerminal ? Color(.systemBackground) : Color(uiColor: GhosttyTerminalUIView.backgroundColor)
                    SessionBarGlass(fade: items.isEmpty ? nil : page).ignoresSafeArea(edges: [.top, .horizontal])
                }
            }
        }
        .safeAreaInset(edge: .bottom, spacing: 0) {
            VStack(spacing: 0) {
                let continueInTerminal = showingChat && !model.isTerminal && approvalItems.allSatisfy(\.approval.answeredInTerminal) && (model.codexQuestionQueued || model.codexQuestionOpen || model.agent?.status == .blocked || !model.noticeOptions.isEmpty || model.commandShown)
                // Typing in landscape, the hint's row is what the keyboard's predictions bar leaves the approval card.
                // The approval cards and Continue in Terminal already say how to answer.
                if let notice = model.notice ?? (typingInLandscape || !approvalItems.isEmpty || continueInTerminal ? nil : model.blockedHint) {
                    Label(notice, systemImage: "exclamationmark.triangle")
                        .font(.footnote)
                        .foregroundStyle(.warning)
                        .composerNoteGlass()
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .padding(.horizontal)
                        .padding(.top, 6)
                }
                if continueInTerminal {
                    Button("Continue in Terminal", systemImage: "terminal") { showingChat = false }
                        .font(.footnote).buttonStyle(.glass).background(SessionBarDim(), in: .capsule)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .padding(.horizontal)
                        .padding(.top, 6)
                }
                GlassEffectContainer(spacing: 4) {
                    VStack(spacing: 0) {
                        if model.keysShown && (!showingChat || model.isTerminal) { KeyStrip(model: model, typing: $typing) }
                        PromptBar(model: model, typesIntoTerminal: !showingChat || model.isTerminal, typing: $typing)
                    }
                }
            }
            .background {
                // Typing, the terminal keeps Claude Code's input box above the composer, which puts the rows under the box
                // behind the key strip and the composer, where they showed between and through the controls. No feather:
                // the box's bottom rule ends at this edge, and a feather showed half of the row under it.
                if typing && (!showingChat || model.isTerminal) {
                    Color(uiColor: GhosttyTerminalUIView.backgroundColor)
                        .ignoresSafeArea(edges: .bottom)
                        .allowsHitTesting(false)
                }
            }
            // Without a band, touches in the gaps between controls would reach the terminal's swipe and tap gestures.
            .contentShape(.rect)
            .onGeometryChange(for: CGFloat.self) { $0.size.height } action: { composerHeight = $0 }
        }
        .background {
            // The whole screen, bars included: the keyboard must not shrink the cards' cap.
            Color.clear
                .ignoresSafeArea(.keyboard)
                .onGeometryChange(for: CGFloat.self) { $0.size.height + $0.safeAreaInsets.top + $0.safeAreaInsets.bottom } action: {
                    height = $0
                }
                .onGeometryChange(for: CGFloat.self) { $0.size.height } action: { barsRoom = $0 }
        }
        .navigationTitle(title)
        .navigationBarTitleDisplayMode(.inline)
        .toolbar(.hidden, for: .tabBar)
        .toolbar(typingInLandscape ? .hidden : .automatic, for: .navigationBar)
        .toolbar {
            ToolbarItem(placement: .principal) {
                // At accessibility sizes the title heads the session header instead, where it can outgrow the bar, unless
                // approval cards show: there its lines left the command and the reason no room above the decisions.
                if typeSize.isAccessibilitySize && verticalSizeClass != .compact && approvalItems.isEmpty {
                    Color.clear.frame(width: 1, height: 1)
                } else {
                    SessionTitle(title: title)
                }
            }
                .sharedBackgroundVisibility(.hidden)
            ToolbarItem(placement: .primaryAction) {
                if showingChat && !model.isTerminal {
                    Button("Search conversation", systemImage: "magnifyingglass") { searchingChat.toggle() }
                        .tint(.primary)
                } else {
                    // A button-style Toggle fills the bar item when on, and bar buttons pass only their title to accessibility.
                    Button { model.wrapLines.toggle() } label: {
                        Label(model.wrapLines ? "Wrap lines on" : "Wrap lines off", image: "WrapLines")
                    }
                    .tint(model.wrapLines ? Color.accentColor : neutralAccent ? .secondary : .primary)
                }
            }
            ToolbarItem(placement: .primaryAction) {
                Menu("More", systemImage: "ellipsis") {
                    if PullRequestBadge.archiveFirst(model.agent) {
                        archiveButton
                        Divider()
                    }
                    if !model.isTerminal {
                        Button("Git changes", systemImage: "arrow.triangle.branch") { showingChanges = true }
                        Button("Refresh", systemImage: "arrow.clockwise") {
                            Task { await model.refresh() }
                        }
                        .disabled(model.refreshing)
                        Button("Focus on \(machineLabel ?? "machine")", systemImage: "desktopcomputer") {
                            Task { await model.focus() }
                        }
                        if let follows {
                            FollowMenuItem(follows: follows, route: model.route)
                        }
                        switch PullRequestBadge.watchMenu(model.agent) {
                        case .watch:
                            Menu("Watch PR", systemImage: "eye") {
                                ForEach(PullRequestBadge.intervals, id: \.self) { minutes in
                                    Button("Every \(minutes) minutes") {
                                        Task { await model.watchPullRequest(everyMinutes: minutes) }
                                    }
                                }
                            }
                        case .stop(let title):
                            Button(title, systemImage: "eye.slash") {
                                Task { await model.watchPullRequest(everyMinutes: nil) }
                            }
                        case nil:
                            EmptyView()
                        }
                    }
                    Divider()
                    Button("Close pane", systemImage: "xmark.square", role: .destructive) {
                        model.close.begin(.pane)
                    }
                    .tint(.red)
                    .disabled(model.archiving)
                    Button("Close workspace", systemImage: "xmark.rectangle.portrait", role: .destructive) {
                        if let workspaceId = model.agent?.workspaceId ?? model.terminal?.workspaceId {
                            model.close.begin(.workspace(id: workspaceId))
                        }
                    }
                    .tint(.red)
                    .disabled(model.archiving || (model.agent == nil && model.terminal == nil))
                    if !PullRequestBadge.archiveFirst(model.agent) {
                        archiveButton
                    }
                }
                .tint(.primary)
            }
        }
        .sheet(isPresented: $showingChanges) {
            GitChangesView(agent: model, sent: { showingChanges = false }, review: model.gitReview) { route in
                showingChanges = false
                if route != model.route { switchAgent(route) }
            }
        }
        .sheet(item: $model.fileLink) { link in
            FileViewer(agent: model, link: link)
        }
        // Presented here, not from the card: the band takes the dark scheme over the terminal, and the sheet follows the appearance.
        .sheet(item: reviewing) { item in
            if let approvals { ApprovalReview(model: approvals, item: item, task: title, showsMachine: showsMachine, sendAnswer: model.typeAnswer) }
        }
        .onChange(of: blocked, initial: true) { _, blocked in model.blocked = blocked }
        .onChange(of: model.agent, initial: true) { _, agent in
            guard !recordedOpen, let agent else { return }
            recordedOpen = true
            opened(agent)
        }
        .closeConfirmation($model.close, archiving: model.archiving) { await model.performClose() }
        .onChange(of: model.archiveMessage) { _, message in
            guard let message else { return }
            archived(message)
            model.finishArchive()
        }
        .onChange(of: model.closed) { _, closed in
            if closed { dismiss() }
        }
        .task { await model.run() }
        .onReceive(NotificationCenter.default.publisher(for: UIResponder.keyboardWillShowNotification)) { note in
            // A hardware keyboard shows only its shortcut bar, or a frame off screen.
            guard let screen = note.object as? UIScreen, let frame = note.userInfo?[UIResponder.keyboardFrameEndUserInfoKey] as? CGRect else { return }
            keyboardShown = frame.intersection(screen.bounds).height > 100
        }
        .onReceive(NotificationCenter.default.publisher(for: UIResponder.keyboardWillHideNotification)) { _ in keyboardShown = false }
        .onAppear { model.reloadGestures() }
        .onChange(of: showingChat) { typing = false }
        .onChange(of: scenePhase) { _, phase in
            model.dictation.scenePhaseChanged(to: phase)
            if phase != .active { model.saveDraft() }
        }
        .onDisappear {
            model.cancelUpload()
            model.dictation.cancel()
            model.saveDraft()
        }
    }

    private func perform(_ action: GestureAction) {
        switch action {
        case .none: break
        // A paste never sends: its new lines are typed as spaces, not Return.
        case .paste: if let text = UIPasteboard.general.string { model.type(text.split(whereSeparator: \.isNewline).joined(separator: " ")) }
        case .escape: model.tap(.esc)
        }
    }
}

extension View {
    /// Asks to confirm `close` once it begins; `perform` only runs from that confirmation.
    func closeConfirmation(_ close: Binding<CloseConfirmation>, archiving: Bool = false, archiveNotice: String? = nil, perform: @escaping @MainActor () async -> Void) -> some View {
        modifier(CloseDialogs(close: close, archiving: archiving, archiveNotice: archiveNotice, perform: perform))
    }
}

private struct CloseDialogs: ViewModifier {
    @Binding var close: CloseConfirmation
    let archiving: Bool
    let archiveNotice: String?
    let perform: @MainActor () async -> Void
    @State private var asking = false

    func body(content: Content) -> some View {
        content
            .safeAreaInset(edge: .top) {
                if archiving || archiveNotice != nil {
                    HStack(spacing: 10) {
                        if archiving {
                            ProgressView().controlSize(.small)
                            Text("Archiving task and cleaning up branches…")
                        } else if let archiveNotice {
                            Label(archiveNotice, systemImage: "checkmark.circle.fill")
                                .foregroundStyle(.green)
                        }
                    }
                    .font(.footnote)
                    .padding(12)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .glassEffect(.regular, in: .rect(cornerRadius: 12))
                    .padding(.horizontal)
                    .padding(.vertical, 6)
                    .allowsHitTesting(false)
                }
            }
            .onChange(of: close.step) { _, step in
                if case .asking = step { asking = true }
            }
            .onChange(of: asking) { _, shown in
                if !shown, case .asking = close.step { close.cancel() }
            }
            .confirmationDialog(title, isPresented: $asking, titleVisibility: .visible) {
                Button(action, role: .destructive) {
                    close.advance()
                    Task { await perform() }
                }
                Button("Cancel", role: .cancel) { close.cancel() }
            } message: {
                Text(message)
            }
    }

    private var title: String {
        switch close.target {
        case .pane: "Close this pane?"
        case .workspace: "Close this workspace?"
        case .archive: "Archive this task?"
        case nil: ""
        }
    }

    private var action: String {
        switch close.target {
        case .pane: "Close pane"
        case .workspace: "Close workspace"
        case .archive: "Archive"
        case nil: ""
        }
    }

    private var message: String {
        switch close.target {
        case .pane: "The agent and its shell on the machine are ended."
        case .workspace: "Every agent and shell in the workspace on the machine is ended."
        case .archive:
            "Ends this task and deletes its worktree, including uncommitted changes. Branch cleanup may also remove other merged worktrees."
        case nil: ""
        }
    }
}

private struct AgentHeader: View {
    let model: AgentModel
    let title: String
    let approving: Bool
    let machineLabel: String?
    @Binding var showingChat: Bool
    @Environment(\.dynamicTypeSize) private var typeSize
    @Environment(\.verticalSizeClass) private var verticalSizeClass
    // Landscape keeps the title in the bar: a header row more left the session two lines of the screen.
    private var stacked: Bool { typeSize.isAccessibilitySize && verticalSizeClass != .compact }

    var body: some View {
        VStack(spacing: 6) {
            if stacked && !approving {
                SessionTitle(title: title, inHeader: true).frame(maxWidth: .infinity, alignment: .leading)
            }
            // One row, like the navigation bar it sits under, its text no larger than the largest standard size: at
            // accessibility sizes the row pushed the screen past both edges, and two rows under the title took a third
            // of the screen. Two rows only when even that does not fit (a pull request badge on a narrow phone).
            ViewThatFits(in: .horizontal) {
                statusRow(split: false)
                statusRow(split: true)
            }
            .dynamicTypeSize(...DynamicTypeSize.xxxLarge)
            if model.link == .connecting {
                HStack(spacing: 6) {
                    ProgressView().controlSize(.mini)
                    Text("reconnecting").font(.caption).foregroundStyle(.primary)
                }
                .frame(maxWidth: .infinity, alignment: .trailing)
            } else if let link = model.link, link != .connected {
                Text(model.linkError ?? link.label)
                    .font(.caption)
                    .foregroundStyle(.warning)
                    .lineLimit(1)
                    .frame(maxWidth: .infinity, alignment: .trailing)
            }
        }
        .padding(.horizontal)
        .padding(.vertical, 6)
    }

    private func statusRow(split: Bool) -> some View {
        let row = split ? AnyLayout(VStackLayout(alignment: .leading, spacing: 6)) : AnyLayout(HStackLayout(spacing: 8))
        return row {
            HStack(spacing: 8) {
                if let agent = model.agent {
                    HStack(spacing: 6) {
                        StatusIcon(state: agent.status, dimsIdle: false)
                        if !approving {
                            TimelineView(.periodic(from: .now, by: 1)) { context in
                                Text(Elapsed.string(sinceMs: agent.statusSinceMs, now: context.date))
                                    .font(.caption.monospacedDigit())
                                    .foregroundStyle(.primary)
                            }
                        }
                    }
                    .fixedSize()
                }
                if let machineLabel {
                    Text(verbatim: machineLabel)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                        .truncationMode(.middle)
                        // Truncated first, so the row never splits for it.
                        .frame(minWidth: 0, idealWidth: 0, maxWidth: .infinity, alignment: .leading)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            HStack(spacing: 8) {
                if let agent = model.agent {
                    if let pullRequest = agent.pullRequest {
                        PullRequestBadge(pullRequest: pullRequest, watch: agent.prWatch) { text in
                            model.insert(PromptShortcut(text: text))
                        }
                    }
                    HStack(spacing: 6) {
                        if let kind = agent.kind {
                            AgentKindLabel(kind: kind, iconOnly: true).font(.caption).foregroundStyle(.secondary)
                        }
                        if let left = agent.contextLeft, ContextRing.shows(left) {
                            ContextRing(left: left)
                        }
                    }
                    .fixedSize()
                } else {
                    if model.isTerminal {
                        AgentKindLabel(kind: "terminal").font(.caption).foregroundStyle(.primary).fixedSize()
                        Label(model.terminalLocked ? "Locked" : "Unlocked", systemImage: model.terminalLocked ? "lock.fill" : "lock.open")
                            .font(.caption)
                            .foregroundStyle(.primary)
                            .fixedSize()
                    }
                }
                if split { Spacer(minLength: 0) }
                if !model.isTerminal {
                    // No glass backing here: an extra glass layer beside other glass left keyboard animations unfinished after a picker closed.
                    GlassPicker(title: "Session view", selection: $showingChat, options: [("Chat", true), ("Terminal", false)],
                                icons: [UIImage(systemName: "bubble.left.and.bubble.right"), UIImage(systemName: "terminal")])
                        .controlSize(.small)
                }
            }
            .layoutPriority(1)
        }
    }
}

private struct SessionBarGlass: View {
    /// The page under the approval band, which content fades into below its edge.
    var fade: Color?
    @Environment(\.colorScheme) private var colorScheme
    // Grows with the text: a fixed 8 points cut large text running under the edge in half, blurred above and sharp below.
    @ScaledMetric private var feather: CGFloat = 8

    var body: some View {
        // The glass rim along the bottom edge drew a hairline seam over the content: it lies past the mask,
        // and the glass fades out over its last points rather than ending on a hard cut.
        // Light glass over a white chat is nearly white itself: a faint gray tint gives the band a visible edge.
        Color.clear.glassEffect(.regular, in: .rect)
            .overlay(colorScheme == .light ? Color.black.opacity(0.04) : .clear)
            .padding(.bottom, -feather)
            .mask {
                VStack(spacing: 0) {
                    Rectangle()
                    LinearGradient(colors: [.black, .clear], startPoint: .top, endPoint: .bottom).frame(height: feather)
                }
            }
            // Under the taller band, a line of chat crossing its edge read cut in half, blurred above and sharp below.
            .background(alignment: .bottom) {
                if let fade {
                    LinearGradient(stops: [.init(color: fade, location: 0), .init(color: fade, location: 0.4), .init(color: fade.opacity(0), location: 1)],
                                   startPoint: .top, endPoint: .bottom)
                        .frame(height: 3 * feather)
                        .offset(y: 2 * feather)
                        .allowsHitTesting(false)
                }
            }
    }
}

// Apple's guidance for clear glass: a dimming layer behind it keeps text legible over bright or busy content.
struct SessionBarDim: ShapeStyle {
    func resolve(in environment: EnvironmentValues) -> Color {
        environment.colorScheme == .dark ? .black.opacity(0.7) : .white.opacity(0.8)
    }
}

struct SubagentList: View {
    let subagents: [SubagentSummary]
    @Environment(\.dynamicTypeSize) private var typeSize

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            ForEach(Array(subagents.enumerated()), id: \.offset) { _, subagent in
                TimelineView(.periodic(from: Date(timeIntervalSince1970: Double(subagent.startedAtMs / 1000)), by: 60)) { context in
                    let usage = Text(verbatim: subagent.usage(now: context.date))
                        .monospacedDigit()
                        .foregroundStyle(.primary)
                        .lineLimit(1)
                    VStack(alignment: .leading, spacing: 0) {
                        HStack(spacing: 6) {
                            Image(systemName: "circle").font(.system(size: 7)).foregroundStyle(.primary).accessibilityHidden(true)
                            Text(verbatim: subagent.title).lineLimit(1)
                            Spacer(minLength: 4)
                            if !typeSize.isAccessibilitySize { usage.fixedSize() }
                        }
                        if typeSize.isAccessibilitySize { usage }
                    }
                    .accessibilityElement(children: .combine)
                }
            }
        }
        .font(.caption)
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}

extension SubagentSummary {
    func elapsed(now: Date) -> String {
        let minutes = max(0, Int(now.timeIntervalSince1970) - Int(startedAtMs / 1000)) / 60
        return switch minutes {
        case 0: "<1m"
        case ..<60: "\(minutes)m"
        default: "\(minutes / 60)h \(minutes % 60)m"
        }
    }

    /// "1h 11m · ↓ 270.8k": ↓ once the subagent has called a tool, ↑ before.
    func usage(now: Date) -> String {
        let elapsed = elapsed(now: now)
        guard tokens > 0 else { return elapsed }
        return "\(elapsed) · \(calledTools ? "↓" : "↑") \(Self.count(tokens))"
    }

    static func count(_ n: UInt64) -> String {
        let (value, unit) = n < 1000 ? (Double(n), "") : n < 999_950 ? (Double(n) / 1000, "k") : (Double(n) / 1_000_000, "m")
        let tenths = (value * 10).rounded()
        return (tenths.truncatingRemainder(dividingBy: 10) == 0 ? "\(Int(tenths / 10))" : String(format: "%.1f", tenths / 10)) + unit
    }
}

/// The inline title on up to two lines, cut after the last character that fits rather than ending in an ellipsis.
/// In the session header, at accessibility sizes, it leads and grows with the text past what the bar holds, at title 3
/// size: subheadline and headline came out about as large as the status row's caption there.
struct SessionTitle: UIViewRepresentable {
    let title: String
    var inHeader = false

    func makeUIView(context: Context) -> UILabel {
        let label = UILabel()
        label.numberOfLines = 2
        label.lineBreakMode = .byWordWrapping
        label.textAlignment = inHeader ? .natural : .center
        label.font = inHeader
            ? UIFontMetrics(forTextStyle: .title3).scaledFont(for: .systemFont(ofSize: 20, weight: .bold))
            : UIFontMetrics(forTextStyle: .subheadline).scaledFont(for: .systemFont(ofSize: 15, weight: .bold), maximumPointSize: 19)
        label.adjustsFontForContentSizeCategory = true
        label.accessibilityTraits = .header
        return label
    }

    func updateUIView(_ label: UILabel, context: Context) {
        label.text = title
        label.accessibilityLabel = title
    }

    func sizeThatFits(_ proposal: ProposedViewSize, uiView: UILabel, context: Context) -> CGSize? {
        let width = proposal.width ?? uiView.intrinsicContentSize.width
        let size = uiView.sizeThatFits(CGSize(width: width, height: .greatestFiniteMagnitude))
        return CGSize(width: min(size.width, width), height: size.height)
    }
}

/// GhosttyTerminalUIView with a refresh control; `.refreshable` does not reach a UIKit scroll view.
private struct AgentTerminal: UIViewRepresentable {
    let ansi: String
    let wraps: Bool
    let fontSize: Double
    let gestures: TerminalGestures
    let perform: @MainActor (GestureAction) -> Void
    let resized: @MainActor (Double) -> Void
    let resizeEnded: @MainActor () -> Void
    let quote: (@MainActor (String) -> Void)?
    let openFile: (@MainActor (String) -> Void)?
    let neighbor: ((Int) -> AgentRoute?)?
    let switchAgent: (AgentRoute) -> Void
    let obscuredTop: CGFloat
    let obscuredBottom: CGFloat
    @Binding var typing: Bool
    let type: @MainActor (String) -> Void
    let deleteBackward: @MainActor () -> Void
    let inputAgent: String?
    let tapInput: (@MainActor (Int, Int) -> Void)?
    let refresh: @MainActor () async -> Void

    func makeUIView(context: Context) -> GhosttyTerminalUIView {
        let view = GhosttyTerminalUIView(fontSize: fontSize)
        // obscuredTop and obscuredBottom already include the bars and the device's safe area.
        view.contentInsetAdjustmentBehavior = .never
        // With the canvas's own 8, text starts at the chrome's 16-point gutter; grid previews keep 8.
        view.contentInset.left = 8
        view.contentInset.right = 8
        view.contentOffset.x = -8
        view.topEdgeEffect.isHidden = true
        let control = UIRefreshControl()
        let coordinator = context.coordinator
        control.addAction(
            UIAction { [weak control] _ in
                Task {
                    await coordinator.refresh?()
                    control?.endRefreshing()
                }
            },
            for: .valueChanged
        )
        view.refreshControl = control
        return view
    }

    func updateUIView(_ view: GhosttyTerminalUIView, context: Context) {
        context.coordinator.refresh = refresh
        view.wraps = wraps
        view.fontSize = fontSize
        view.obscuredTop = obscuredTop
        view.obscuredBottom = obscuredBottom
        let (gestures, perform, resized, resizeEnded) = (gestures, perform, resized, resizeEnded)
        view.onDoubleTap = gestures.doubleTap == .none ? nil : { perform(gestures.doubleTap) }
        view.onTripleTap = gestures.tripleTap == .none ? nil : { perform(gestures.tripleTap) }
        view.onPinch = gestures.pinchResizesText ? { resized($0) } : nil
        view.onPinchEnded = { resizeEnded() }
        view.onQuote = quote.map { quote in { quote($0) } }
        view.filePath = openFile == nil ? nil : { FileLinks.path(selected: $0) }
        view.onOpenFile = openFile.map { openFile in { openFile($0) } }
        if gestures.swipeSwitchesAgents, let neighbor {
            let switchAgent = switchAgent
            let offset = { (swipe: TerminalSwipe) in swipe == .left ? 1 : -1 }
            view.canSwipe = { neighbor(offset($0)) != nil }
            view.onSwipe = { if let next = neighbor(offset($0)) { switchAgent(next) } }
        } else {
            view.canSwipe = nil
            view.onSwipe = nil
        }
        let keyboard = $typing
        view.inputAgent = inputAgent
        view.onTapInput = tapInput.map { tapInput in
            { index, count in
                tapInput(index, count)
                keyboard.wrappedValue = true
            }
        }
        if !ansi.isEmpty {
            view.show(ansiSnapshot: ansi)
        }
        let (typing, type, deleteBackward) = ($typing, type, deleteBackward)
        view.onTap = { typing.wrappedValue = true }
        view.onInsertText = { type($0) }
        view.onDeleteBackward = { deleteBackward() }
        // The state follows the keyboard, so one UIKit takes away is not asked for again.
        view.onTypingChanged = { shown in Task { @MainActor in typing.wrappedValue = shown } }
        if self.typing != view.typing {
            let wanted = self.typing
            Task { @MainActor in view.typing = wanted }
        }
    }

    func makeCoordinator() -> Coordinator { Coordinator() }

    @MainActor
    final class Coordinator {
        var refresh: (@MainActor () async -> Void)?
    }
}

/// The agent exited, or the pane is a shell: what it is now, and Unlock when it is a locked shell.
private struct PaneCard: View {
    let model: AgentModel
    let notice: String

    var body: some View {
        HStack(spacing: 12) {
            Text(notice).font(.footnote).frame(maxWidth: .infinity, alignment: .leading)
            if model.canUnlock || model.unlocking {
                Button {
                    Task { await model.unlock() }
                } label: {
                    if model.unlocking {
                        ProgressView().controlSize(.small)
                    } else {
                        Label("Unlock with Face ID", systemImage: "faceid")
                    }
                }
                .buttonStyle(.glassProminent)
                .onAccentFill()
                .controlSize(.small)
                .disabled(model.unlocking)
            }
        }
        .padding(.horizontal)
        .padding(.vertical, 8)
        .background(.fill.quaternary)
    }
}

private struct KeyStrip: View {
    let model: AgentModel
    @Binding var typing: Bool
    @State private var shortcuts: StripShortcuts
    @State private var editingShortcuts = false

    init(model: AgentModel, typing: Binding<Bool>) {
        self.model = model
        _typing = typing
        _shortcuts = State(initialValue: StripShortcuts.load(from: model.savesPreferences ? StripShortcuts.file : nil))
    }

    var body: some View {
        // The chips' touch targets fill most of the gaps above and below them.
        VStack(spacing: 1) {
            HStack(spacing: 8) {
                mainKeys
                CursorJoystick(model: model)
            }
            ScrollView(.horizontal) {
                HStack(spacing: 6) {
                    // shift-left answers a queued Codex question; other agents never take it.
                    ForEach(AgentKey.combos.filter { $0 != .shiftLeft || model.accepts($0) }, id: \.self) { key in
                        StripChip { model.run(StripShortcut(key: key.name)) } label: { KeyGlyphs(key: key).font(.system(size: 20)) }
                            .accessibilityLabel(key.accessibilityName)
                            .disabled(!model.canTap(key))
                    }
                    ForEach(shortcuts.items.filter { $0.applies(to: model.shortcutKind) }) { shortcut in
                        StripChip(literal: shortcut.name.isEmpty && shortcut.agentKey == nil) { model.run(shortcut) } label: {
                            if shortcut.name.isEmpty, let key = shortcut.agentKey {
                                KeyGlyphs(key: key).font(.system(size: 20))
                            } else {
                                Text(shortcut.name.isEmpty ? shortcut.text : shortcut.name)
                            }
                        }
                        .accessibilityLabel(shortcut.name.isEmpty ? shortcut.agentKey?.accessibilityName ?? shortcut.text : shortcut.name)
                        .disabled(!model.canRun(shortcut))
                    }
                    StripChip { editingShortcuts = true } label: { Image(systemName: "plus") }
                        .disabled(!model.savesPreferences)
                        .accessibilityLabel("Edit key shortcuts")
                }
            }
            .scrollIndicators(.hidden)
        }
        .padding(.horizontal)
        .padding(.top, 8)
        .padding(.bottom, -7)
        // At accessibility sizes even six keys to a row cut glyphs and pushed the screen past both edges.
        .dynamicTypeSize(...DynamicTypeSize.xxxLarge)
        .sensoryFeedback(.impact(weight: .light), trigger: model.keyTaps)
        .onDisappear { model.modifier = nil }
        .sheet(isPresented: $editingShortcuts, onDismiss: { shortcuts = StripShortcuts.load(from: StripShortcuts.file) }) {
            NavigationStack {
                StripShortcutsView { editingShortcuts = false }
            }
        }
    }

    private var mainKeys: some View {
        HStack(spacing: 0) {
            ForEach([KeyModifier.control, .shift], id: \.self) { modifier in
                let selected = model.modifier == modifier
                KeyCap(selected: selected) { model.select(modifier) } label: {
                    Image(systemName: modifier == .control ? "control" : "shift")
                }
                .accessibilityLabel(modifier.accessibilityName)
                .accessibilityAddTraits(selected ? .isSelected : [])
                .disabled(!model.acceptsKeys)
                Divider().frame(height: 18)
            }
            ForEach(AgentKey.strip, id: \.self) { key in
                KeyCap {
                    guard model.press(key) != nil, key == .enter, !model.keepsKeyboard(after: "\n") else { return }
                    typing = false
                } label: { KeyGlyphs(key: key) }
                .accessibilityLabel(key.accessibilityName)
                .disabled(!model.canPress(key))
                if key != AgentKey.strip.last { Divider().frame(height: 18) }
            }
        }
        .padding(.horizontal, 4)
        .glassEffect(.regular.interactive(), in: .capsule)
        .background(SessionBarDim(), in: .capsule)
    }
}

/// One key in the main keys' shared glass capsule.
private struct KeyCap<Label: View>: View {
    var selected = false
    let action: () -> Void
    @ViewBuilder let label: Label
    @Environment(\.isEnabled) private var isEnabled
    @Environment(\.appAccent) private var appAccent

    var body: some View {
        Button(action: action) {
            label
                .font(.system(size: 22))
                .lineLimit(1)
                .foregroundStyle(selected ? appAccent : .primary)
                .padding(.horizontal, 6)
                .frame(maxWidth: .infinity, minHeight: 44)
                .background {
                    if selected { Capsule().fill(appAccent.opacity(0.2)).padding(.vertical, 6) }
                }
                .contentShape(.rect)
                .opacity(isEnabled ? 1 : 0.4)
        }
        .buttonStyle(.plain)
    }
}

/// A combination or saved shortcut in the strip's scrolling second row.
private struct StripChip<Label: View>: View {
    /// Text typed as is, which reads as code.
    var literal = false
    let action: () -> Void
    @ViewBuilder let label: Label
    @Environment(\.isEnabled) private var isEnabled

    var body: some View {
        Button(action: action) {
            label
                .font(literal ? .callout.monospaced().weight(.medium) : .callout.weight(.medium))
                .lineLimit(1)
                .padding(.horizontal, 11)
                .frame(minHeight: 30)
                .opacity(isEnabled ? 1 : 0.4)
                .glassEffect(.regular.interactive(), in: .capsule)
                .background(SessionBarDim(), in: .capsule)
                // A 44-point target around the compact capsule, inside the scroll view, which takes no touches past its bounds.
                .padding(.vertical, 7)
                .contentShape(.rect)
        }
        .buttonStyle(.plain)
    }
}

/// Dragged past its dead zone, sends that arrow, then repeats it while held.
private struct CursorJoystick: View {
    let model: AgentModel
    var diameter: CGFloat = 44
    @State private var direction: AgentKey?
    @State private var pressed = false
    @State private var repeating: Task<Void, Never>?
    @Environment(\.appAccent) private var appAccent

    var body: some View {
        ZStack {
            if let direction {
                JoystickSector(direction: direction).fill(appAccent)
            }
            if pressed {
                Circle().fill(.primary).frame(width: 8, height: 8)
            } else {
                Image(systemName: "dot.arrowtriangles.up.right.down.left.circle").font(.system(size: diameter - 18))
            }
        }
        .frame(width: diameter, height: diameter)
        .clipShape(.circle)
        .glassEffect(.regular.interactive(), in: .circle)
        .background(SessionBarDim(), in: .circle)
        .contentShape(Circle().inset(by: (diameter - 44) / 2))
        .opacity(model.acceptsKeys ? 1 : 0.4)
        .gesture(
            DragGesture(minimumDistance: 0)
                .onChanged { value in
                    pressed = true
                    let key = AgentKey.arrow(for: value.translation)
                    guard key != direction else { return }
                    direction = key
                    repeating?.cancel()
                    guard let key else { return }
                    repeating = Task {
                        var delay = Duration.milliseconds(300)
                        while !Task.isCancelled {
                            send(key)
                            try? await Task.sleep(for: delay)
                            delay = .milliseconds(83)
                        }
                    }
                }
                .onEnded { _ in stop() }
        )
        .disabled(!model.acceptsKeys)
        .onDisappear(perform: stop)
        .accessibilityElement()
        .accessibilityLabel("Arrow joystick")
        .accessibilityActions {
            ForEach([AgentKey.up, .down, .left, .right], id: \.self) { key in
                Button(key.accessibilityName) { send(key) }
            }
        }
    }

    private func send(_ key: AgentKey) {
        if model.canPress(key) { model.press(key) }
    }

    private func stop() {
        repeating?.cancel()
        repeating = nil
        direction = nil
        pressed = false
    }
}

private struct JoystickSector: Shape {
    let direction: AgentKey

    func path(in rect: CGRect) -> Path {
        let center = CGPoint(x: rect.midX, y: rect.midY)
        let radius = min(rect.width, rect.height) / 2
        let angle: Double = switch direction {
        case .right: 0
        case .down: 90
        case .left: 180
        default: 270
        }
        var path = Path()
        path.move(to: center)
        for step in 0...12 {
            let radians = (angle - 45 + Double(step) * 7.5) * .pi / 180
            path.addLine(to: CGPoint(x: center.x + radius * cos(radians), y: center.y + radius * sin(radians)))
        }
        path.closeSubpath()
        return path
    }
}

/// A key's SF Symbols, the modifier's first; a control letter ends with the letter, which has no symbol.
private struct KeyGlyphs: View {
    let key: AgentKey

    var body: some View {
        HStack(spacing: 3) {
            switch key {
            case .esc: Image(systemName: "escape")
            case .enter: Image(systemName: "return")
            case .up: Image(systemName: "arrow.up")
            case .down: Image(systemName: "arrow.down")
            case .left: Image(systemName: "arrow.left")
            case .right: Image(systemName: "arrow.right")
            case .tab: Image(systemName: "arrow.right.to.line")
            case .backspace: Image(systemName: "delete.left")
            case .shiftTab: Image(systemName: "shift"); Image(systemName: "arrow.right.to.line")
            case .shiftLeft: Image(systemName: "shift"); Image(systemName: "arrow.left")
            case .ctrlEnter: Image(systemName: "control"); Image(systemName: "return")
            case .ctrlC, .ctrlA, .ctrlB, .ctrlE, .ctrlK, .ctrlL, .ctrlU, .ctrlW:
                Image(systemName: "control"); Text(key.name.dropFirst(5).uppercased())
            }
        }
    }
}

extension AgentKey {
    /// The arrow a joystick dragged by `offset` points to; nil inside its dead zone.
    static func arrow(for offset: CGSize, deadZone: CGFloat = 12) -> AgentKey? {
        guard hypot(offset.width, offset.height) >= deadZone else { return nil }
        if abs(offset.width) > abs(offset.height) { return offset.width > 0 ? .right : .left }
        return offset.height > 0 ? .down : .up
    }
}

private struct PromptBar: View {
    @Bindable var model: AgentModel
    let typesIntoTerminal: Bool
    @Binding var typing: Bool
    @FocusState private var editing: Bool
    @State private var keptDraft: String?
    @State private var dictationExpanded = false
    @State private var dictationShown = false
    @State private var selection: TextSelection?
    @State private var pickingPhoto = false
    @State private var pickingFile = false
    @State private var pickingPrompt = false
    @State private var droppingLetter: String?
    @State private var copiedAt: Date?
    @State private var returns = 0
    @Environment(\.verticalSizeClass) private var verticalSizeClass
    @Environment(\.appAccent) private var appAccent

    private func typed(_ text: String) {
        let old = model.draft
        guard model.controlTyped(from: old, to: text) else {
            model.typed(text)
            return
        }
        // The field keeps showing the letter until the bound value changes: it takes the letter, and the
        // update that saw it drops it again (onChange), so the two writes never land in one update.
        model.draft = text
        droppingLetter = old
    }

    private func copyPrompt() {
        guard !model.draft.isEmpty else { return }
        UIPasteboard.general.string = model.draft
        copiedAt = .now
    }

    private var reading: Bool {
        !editing && !model.draft.isEmpty && !model.dictation.isActive
    }

    private var expanded: Bool {
        editing || model.dictation.isActive || dictationShown || !model.attachments.isEmpty || (dictationExpanded && !model.draft.isEmpty)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            if let error = model.promptError {
                Text(error).font(.footnote).foregroundStyle(.red).composerNoteGlass()
            }
            if model.retrying {
                Text("Unreachable, retrying…").font(.footnote).foregroundStyle(.secondary).composerNoteGlass()
            }
            if let upload = model.upload {
                UploadChip(upload: upload) { model.cancelUpload() }
            }
            if let problem = model.dictation.problem {
                DictationProblemRow(problem: problem).composerNoteGlass()
            }
            if !typesIntoTerminal && !model.attachments.isEmpty {
                ScrollView(.horizontal) {
                    HStack(spacing: 8) {
                        ForEach(model.attachments) { file in
                            AttachmentThumbnail(file: file) { model.remove(file) }
                        }
                    }
                }
                .disabled(model.sendingPrompt)
                .scrollIndicators(.hidden)
            }
            if typesIntoTerminal {
                HStack(spacing: 8) {
                    if !model.answering && !model.isTerminal { attachmentMenu }
                    keysButton
                    Spacer(minLength: 0)
                    if !model.isTerminal {
                        DictationControl(dictation: model.dictation, disabled: model.sendingPrompt || !model.savesPreferences, shown: $dictationShown) {
                            keptDraft = model.draft
                            model.draft = ""
                            if model.startDictation() == nil { restoreDraft() }
                        }
                        .background(SessionBarDim(), in: .capsule)
                    }
                    if !model.keysShown { CursorJoystick(model: model, diameter: 38) }
                    Button {
                        typing.toggle()
                    } label: {
                        Image(systemName: typing ? "keyboard.chevron.compact.down" : "keyboard")
                            .font(.system(size: 20))
                            .frame(width: 28, height: 28)
                    }
                    .buttonStyle(.glass)
                    .tint(.primary)
                    .buttonBorderShape(.circle)
                    .controlSize(.small)
                    .contentShape(.interaction, Circle().inset(by: -3))
                    .background(SessionBarDim(), in: .circle)
                    .accessibilityLabel(typing ? "Hide keyboard" : "Keyboard")
                    if !model.keysShown { returnButton }
                }
            } else {
                PromptInputLayout(expanded: expanded && (verticalSizeClass != .compact || model.dictation.isActive || dictationShown)) {
                    HStack(spacing: 8) {
                        if !model.answering && !model.isTerminal { attachmentMenu }
                    }
                    VStack(alignment: .leading, spacing: 8) {
                        HStack(alignment: .bottom, spacing: 8) {
                            let placeholder = model.answering ? "Type an answer" : "Prompt the agent"
                            TextField(
                                placeholder, text: Binding(get: { model.draft }, set: typed),
                                selection: $selection, prompt: Text(placeholder).foregroundStyle(.metadata), axis: .vertical
                            )
                                .lineLimit(expanded || model.showsMacDraft ? 1...(verticalSizeClass == .compact ? 2 : 5) : 1...1)
                                .focused($editing)
                                .disabled(model.dictation.isActive)
                                .foregroundStyle(reading ? Color.clear : Color.primary)
                                .overlay(alignment: .trailing) {
                                    if reading {
                                        DraftReader(text: model.draft)
                                            .onTapGesture { editing = true }
                                            .onLongPressGesture(perform: copyPrompt)
                                            .accessibilityHidden(true)
                                    }
                                    if copiedAt != nil {
                                        CopyIcon(label: "Copy prompt", copiedAt: $copiedAt)
                                            .allowsHitTesting(false)
                                    }
                                }
                                .accessibilityAction(named: "Copy prompt", copyPrompt)
                            if expanded && !model.draft.isEmpty {
                                Button {
                                    if model.dictation.isActive {
                                        model.dictation.clear()
                                    } else {
                                        model.typed("")
                                    }
                                } label: {
                                    Image(systemName: "xmark.circle.fill")
                                        .font(.system(size: 17))
                                        .foregroundStyle(model.dictation.clearArmed ? Color.red : Color.secondary)
                                        .frame(width: 36, height: 36)
                                        .contentShape(Rectangle().inset(by: -4))
                                }
                                .buttonStyle(.plain)
                                .disabled(model.sendingPrompt || model.dictation.phase == .finishing)
                                .accessibilityLabel("Clear text")
                                .accessibilityValue(model.dictation.clearArmed ? "Tap again to clear" : "")
                            }
                        }
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(.horizontal, 12)
                    .padding(.vertical, 8)
                    .glassEffect(.regular.interactive(), in: .rect(cornerRadius: 18))
                    .background(SessionBarDim(), in: .rect(cornerRadius: 18))
                    ZStack {
                        DictationControl(dictation: model.dictation, disabled: model.sendingPrompt || !model.savesPreferences, shown: $dictationShown) {
                            dictationExpanded = model.startDictation(at: editing ? selection : nil) != nil
                            editing = false
                        }
                        .background(SessionBarDim(), in: .capsule)
                    }
                    HStack(spacing: 8) {
                        if editing {
                            Button {
                                editing = false
                            } label: {
                                Image(systemName: "keyboard.chevron.compact.down")
                                    .font(.system(size: 20))
                                    .frame(width: 28, height: 28)
                            }
                            .buttonStyle(.glass)
                            .tint(.primary)
                            .buttonBorderShape(.circle)
                            .controlSize(.small)
                            .contentShape(.interaction, Circle().inset(by: -3))
                            .background(SessionBarDim(), in: .circle)
                            .accessibilityLabel("Hide keyboard")
                        }
                        Button {
                            if model.retrying {
                                model.stopRetry()
                                return
                            }
                            if !model.keepsKeyboard {
                                editing = false
                            }
                            Task { await model.sendPrompt() }
                        } label: {
                            if model.sendingPrompt {
                                ProgressView().tint(appAccent).frame(width: 28, height: 28)
                            } else {
                                Image(systemName: "paperplane.fill").font(.system(size: 17)).frame(width: 28, height: 28)
                            }
                        }
                        .buttonStyle(.glassProminent)
                        .onAccentFill()
                        // Enabled only so a tap stops the retry, so it keeps the look of a send in progress.
                        .tint(model.retrying ? Color(.systemGray4) : appAccent)
                        .buttonBorderShape(.circle)
                        .controlSize(.small)
                        .contentShape(.interaction, Circle().inset(by: -3))
                        .disabled(!model.canSendPrompt && !model.retrying)
                        .accessibilityLabel(model.retrying ? "Stop retrying" : model.answering ? "Send answer" : "Send")
                    }
                }
            }
        }
        .padding(.horizontal)
        .padding(.vertical, 8)
        .sheet(isPresented: $pickingPrompt) {
            NavigationStack {
                PromptShortcutsView { shortcut in
                    if typesIntoTerminal {
                        model.type(shortcut.text)
                    } else {
                        model.insert(shortcut)
                        editing = true
                    }
                }
            }
        }
        .modifier(AttachmentPickers(pickingPhoto: $pickingPhoto, pickingFile: $pickingFile, slots: model.attachmentSlots) {
            model.attach($0)
        } failed: {
            model.attachFailed($0)
        })
        .onChange(of: model.quotes) {
            selection = TextSelection(insertionPoint: model.draft.endIndex)
            editing = true
        }
        .onChange(of: model.draft) {
            if let draft = droppingLetter {
                droppingLetter = nil
                model.draft = draft
            }
        }
        .onChange(of: model.draft.isEmpty) { _, empty in
            if empty && !model.dictation.isActive { dictationExpanded = false }
        }
        .onChange(of: model.dictation.isActive) { _, active in
            if !active { restoreDraft() }
            if !active && model.draft.isEmpty { dictationExpanded = false }
        }
        // Each uploaded file's path is typed into the terminal rather than held for a prompt.
        .onChange(of: model.attachments.map(\.id)) { old, _ in
            guard typesIntoTerminal else { return }
            for file in model.attachments where !old.contains(file.id) {
                model.type(file.path + " ")
                model.remove(file)
            }
        }
        .onDisappear { dictationExpanded = false }
    }

    private var attachmentMenu: some View {
        AttachmentMenu(
            savedPromptsEnabled: !model.dictation.isActive && !model.sendingPrompt,
            filesEnabled: model.attachmentSlots > 0
        ) { choice in
            switch choice {
            case .savedPrompts: pickingPrompt = true
            case .files: pickingFile = true
            case .photos: pickingPhoto = true
            }
        }
        .disabled(model.upload != nil || model.sendingPrompt || !model.savesPreferences)
        .background(SessionBarDim(), in: Circle().inset(by: AttachmentMenuButton.outset))
    }

    private var keysButton: some View {
        Button {
            model.keysShown.toggle()
        } label: {
            Image(systemName: "command")
                .font(.system(size: 20))
                .frame(width: 28, height: 28)
        }
        .buttonStyle(.glass)
        .tint(model.keysShown ? appAccent : .primary)
        .buttonBorderShape(.circle)
        .controlSize(.small)
        .contentShape(.interaction, Circle().inset(by: -3))
        .background(SessionBarDim(), in: .circle)
        .accessibilityLabel("Keys")
        .accessibilityAddTraits(model.keysShown ? .isSelected : [])
    }

    private var returnButton: some View {
        Button {
            guard model.press(.enter) != nil else { return }
            returns += 1
            if !model.keepsKeyboard(after: "\n") { typing = false }
        } label: {
            Image(systemName: "return")
                .font(.system(size: 20))
                .frame(width: 28, height: 28)
        }
        .buttonStyle(.glass)
        .tint(.primary)
        .buttonBorderShape(.circle)
        .controlSize(.small)
        .contentShape(.interaction, Circle().inset(by: -3))
        .background(SessionBarDim(), in: .circle)
        .disabled(!model.canPress(.enter))
        .sensoryFeedback(.impact(weight: .light), trigger: returns)
        .accessibilityLabel("Return")
    }

    /// Dictation in the terminal view types what was said, then gives the chat draft back.
    private func restoreDraft() {
        guard let kept = keptDraft else { return }
        keptDraft = nil
        let said = model.draft
        model.draft = kept
        model.type(said)
    }
}

/// The unfocused prompt, which a drag scrolls without focusing the field and opening the keyboard. UIKit, not
/// Text, so it wraps like the field: Text moves a word down rather than leave it alone on the last line.
private struct DraftReader: UIViewRepresentable {
    let text: String

    func makeUIView(context: Context) -> UITextView {
        let view = UITextView()
        view.isEditable = false
        view.isSelectable = false
        view.backgroundColor = .clear
        view.textContainerInset = .zero
        view.textContainer.lineFragmentPadding = 0
        view.font = .preferredFont(forTextStyle: .body)
        view.adjustsFontForContentSizeCategory = true
        view.textColor = .label
        return view
    }

    func updateUIView(_ view: UITextView, context: Context) {
        if view.text != text { view.text = text }
    }
}

struct DictationButton: View {
    let dictation: DictationModel
    let disabled: Bool
    let starting: () -> Void

    var body: some View {
        Menu {
            Picker(
                "Dictation language",
                selection: Binding(get: { dictation.language }, set: { dictation.select($0) })
            ) {
                ForEach(DictationLanguage.allCases, id: \.self) { Text($0.label).tag($0) }
            }
            .tint(.primary)
        } label: {
            Image(systemName: "waveform")
                .font(.system(size: 20))
                .frame(width: 28, height: 28)
        } primaryAction: {
            if dictation.isActive {
                dictation.stop()
            } else {
                starting()
            }
        }
        .buttonStyle(.glass)
        .tint(.primary)
        .buttonBorderShape(.circle)
        .controlSize(.small)
        .contentShape(.interaction, Circle().inset(by: -3))
        .disabled(disabled)
        .accessibilityLabel(dictation.isActive ? "Stop dictation" : "Dictate in \(dictation.language.label)")
    }
}

struct DictationControl: View {
    let dictation: DictationModel
    let disabled: Bool
    @Binding var shown: Bool
    let starting: () -> Void
    @State private var grown = false
    @State private var revealed = false
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        ZStack(alignment: .trailing) {
            if dictation.isActive || shown {
                DictationBar(dictation: dictation, grown: grown, revealed: revealed)
            } else {
                DictationButton(dictation: dictation, disabled: disabled, starting: starting)
            }
        }
        .task(id: dictation.isActive) {
            let resize: Animation? = reduceMotion ? nil : .smooth(duration: 0.3)
            if dictation.isActive {
                shown = true
                // Lets a Form row finish resizing for the expanded field first, or that resize animates with the grow.
                try? await Task.sleep(for: .milliseconds(50))
                guard !Task.isCancelled else { return }
                withAnimation(resize) { grown = true }
                withAnimation(.easeOut(duration: 0.15).delay(reduceMotion ? 0 : 0.25)) { revealed = true }
            } else {
                guard grown else {
                    shown = false
                    return
                }
                withAnimation(.easeIn(duration: 0.1)) { revealed = false }
                try? await Task.sleep(for: .milliseconds(100))
                guard !Task.isCancelled else { return }
                withAnimation(resize) { grown = false }
                try? await Task.sleep(for: .milliseconds(reduceMotion ? 0 : 300))
                guard !Task.isCancelled else { return }
                shown = false
            }
        }
        .onDisappear { shown = false }
    }
}

struct DictationBar: View {
    let dictation: DictationModel
    let grown: Bool
    let revealed: Bool

    var body: some View {
        HStack(spacing: grown ? 8 : 0) {
            HStack(spacing: 8) {
                switch dictation.phase {
                case .preparing(let download?):
                    ViewThatFits(in: .horizontal) {
                        HStack(spacing: 8) {
                            Text("Downloading speech model").font(.caption).fixedSize()
                            ProgressView(value: download).frame(width: 60)
                        }
                        Text("Downloading…").font(.caption).lineLimit(1)
                    }
                case .listening:
                    DictationWaveform(levels: dictation.levels)
                case .idle, .preparing(nil), .finishing:
                    ProgressView().controlSize(.mini)
                    Text(dictation.phase == .preparing(download: nil) ? "Starting…" : "Finishing…").font(.caption)
                }
                if dictation.phase != .listening { Spacer(minLength: 0) }
                Button(dictation.language.code) { dictation.select(dictation.language.other) }
                    .font(.caption.monospaced().weight(.semibold))
                    .buttonStyle(.plain)
                    .controlSize(.mini)
                    .disabled(dictation.phase == .finishing)
                    .accessibilityLabel("Dictation language, \(dictation.language.label)")
                    .accessibilityHint("Switches to \(dictation.language.other.label)")
            }
            .frame(width: grown ? nil : 0, height: 28)
            // Not 0: a glass whose only accessible child is a button gives that button the glass's accessibility frame.
            .opacity(revealed ? 1 : 0.001)
            .allowsHitTesting(revealed)
            Button(action: dictation.stop) {
                Image(systemName: "waveform")
                    .font(.system(size: 20))
                    .frame(width: 28, height: 28)
                    .foregroundStyle(.tint)
                    .symbolEffect(.breathe, isActive: dictation.isActive && dictation.phase != .finishing)
            }
            .buttonStyle(.plain)
            .disabled(dictation.phase == .finishing)
            .accessibilityLabel("Stop dictation")
        }
        .padding(.leading, grown ? 7 : 0)
        .padding(5)
        .glassEffect(.regular.interactive(), in: .capsule)
    }
}

struct DictationWaveform: View {
    let levels: [Float]
    @Environment(\.accessibilityReduceTransparency) private var reduceTransparency
    @Environment(\.colorSchemeContrast) private var contrast

    var body: some View {
        Canvas { context, size in
            let count = max(1, Int(size.width / 6))
            let visible = Array(levels.suffix(count))
            let leading = count - visible.count
            let inset = (size.width - CGFloat(count - 1) * 6 - 3) / 2
            var path = Path()
            for index in 0..<count {
                let level = index < leading ? 0 : visible[index - leading]
                let height = max(3, CGFloat(level) * size.height)
                path.addRoundedRect(in: CGRect(x: inset + CGFloat(index) * 6, y: (size.height - height) / 2,
                                               width: 3, height: height), cornerSize: CGSize(width: 1.5, height: 1.5))
            }
            if reduceTransparency || contrast == .increased {
                context.fill(path, with: .color(.accentColor))
            } else {
                context.fill(path, with: .linearGradient(
                    Gradient(colors: [Color.accentColor.opacity(0.65), .accentColor, .accentColor.opacity(0.75)]),
                    startPoint: .zero, endPoint: CGPoint(x: 0, y: size.height)
                ))
            }
        }
        .frame(maxWidth: .infinity)
        .frame(height: 24)
        .accessibilityLabel("Dictating")
    }
}

struct DictationProblemRow: View {
    let problem: DictationProblem
    @Environment(\.openURL) private var openURL

    var body: some View {
        HStack(spacing: 8) {
            Text(problem.message).font(.footnote).foregroundStyle(.red)
            if problem == .microphoneDenied, let settings = URL(string: UIApplication.openSettingsURLString) {
                Button("Open Settings") { openURL(settings) }.font(.footnote)
            }
        }
    }
}

struct AttachmentThumbnail: View {
    let file: AttachedFile
    let remove: () -> Void

    var body: some View {
        Group {
            if let thumbnail = file.thumbnail {
                Image(uiImage: thumbnail)
                    .resizable()
                    .scaledToFill()
            } else {
                VStack(spacing: 4) {
                    Image(systemName: file.symbol).font(.title3).foregroundStyle(.secondary)
                    Text(file.name).font(.caption2).lineLimit(1).truncationMode(.middle).padding(.horizontal, 4)
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity)
                .background(.fill.secondary)
                .dynamicTypeSize(...DynamicTypeSize.xxLarge)
            }
        }
        .frame(width: 64, height: 64)
        .clipShape(RoundedRectangle(cornerRadius: 12))
        .contentShape(RoundedRectangle(cornerRadius: 12))
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(file.name)
        .overlay(alignment: .topTrailing) {
            Button(file.kind == .image ? "Remove image" : "Remove file", systemImage: "xmark", action: remove)
                .labelStyle(.iconOnly)
                .buttonStyle(.glass)
                .buttonBorderShape(.circle)
                .controlSize(.mini)
                .tint(.primary)
                .padding(4)
        }
    }
}

struct UploadChip: View {
    let upload: AttachmentUpload
    let cancel: () -> Void

    var body: some View {
        HStack(spacing: 8) {
            Image(systemName: "paperclip").font(.caption).foregroundStyle(.secondary)
            Text(upload.count > 1 ? "\(upload.name) (\(upload.index) of \(upload.count))" : upload.name)
                .font(.caption)
                .lineLimit(1)
                .truncationMode(.middle)
            if upload.sent == 0 {
                ProgressView()
                Text(upload.total == 0 ? "Preparing…" : "Waiting for machine…")
                    .font(.caption)
            } else {
                ProgressView(value: upload.fraction ?? 0).frame(width: 72)
            }
            Button(action: cancel) {
                Image(systemName: "xmark.circle.fill").foregroundStyle(.secondary)
            }
            .buttonStyle(.plain)
            .accessibilityLabel("Cancel upload")
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 6)
        .glassEffect(.regular.interactive(), in: .capsule)
    }
}

extension View {
    func composerNoteGlass() -> some View {
        padding(.horizontal, 10)
            .padding(.vertical, 6)
            .glassEffect(.regular, in: .rect(cornerRadius: 12))
            .background(SessionBarDim(), in: .rect(cornerRadius: 12))
    }
}
