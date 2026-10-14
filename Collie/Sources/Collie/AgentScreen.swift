import CollieCore
import GhosttyTerminal
import PhotosUI
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
    @State private var height: CGFloat = 0
    @Environment(\.dismiss) private var dismiss
    @Environment(\.scenePhase) private var scenePhase
    @Environment(\.verticalSizeClass) private var verticalSizeClass

    init(
        core: any AgentCore, route: AgentRoute, approvals: ApprovalsModel?, follows: FollowModel?,
        machineLabel: String?, showsMachine: Bool, neighbor: ((Int) -> AgentRoute?)?,
        switchAgent: @escaping (AgentRoute) -> Void, opened: @escaping (AgentSummary) -> Void,
        archived: @escaping (String) -> Void
    ) {
        _model = State(initialValue: AgentModel(core: core, route: route, draftsFile: AgentDrafts.file, viewedFile: GitViewed.file, machineLabel: machineLabel))
        self.approvals = approvals
        self.follows = follows
        self.machineLabel = machineLabel
        self.showsMachine = showsMachine
        self.neighbor = neighbor
        self.switchAgent = switchAgent
        self.opened = opened
        self.archived = archived
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
            if !typingInLandscape {
                AgentHeader(model: model, machineLabel: showsMachine ? machineLabel : nil, showingChat: $showingChat)
            }
            if let notice = model.paneNotice {
                PaneCard(model: model, notice: notice)
            }
            if let approvals {
                let cards = ForEach(approvals.items(machineId: model.route.machineId, terminalId: model.route.terminalId)) { item in
                    ApprovalCard(model: approvals, item: item, sendAnswer: model.typeAnswer)
                        .padding(.horizontal)
                        .background(Color.red.opacity(0.08))
                }
                if verticalSizeClass == .compact {
                    // The cards would push the composer off screen, so they scroll within part of the height.
                    ScrollView { VStack(spacing: 0) { cards } }
                        .frame(maxHeight: height / (typingInLandscape ? 3 : 2))
                        .fixedSize(horizontal: false, vertical: true)
                } else {
                    cards
                }
                if let notice = approvals.notice {
                    Text(notice).font(.footnote).padding(.horizontal).frame(maxWidth: .infinity, alignment: .leading)
                }
            }
            if showingChat && !model.isTerminal {
                ConversationView(model: model.conversation, searching: $searchingChat, load: model.history) { showingChat = false }
                    .environment(model)
            } else {
                AgentTerminal(
                    ansi: model.screen, wraps: model.wrapLines, fontSize: model.fontSize, gestures: model.gestures,
                    perform: perform, resized: { model.fontSize = $0 }, resizeEnded: model.saveFontSize,
                    quote: model.isTerminal ? nil : model.quote,
                    // Leaving the screen cancels an upload in flight and hides how a send ends; a draft and its uploaded files are kept.
                    neighbor: model.upload == nil && !model.sendingPrompt ? neighbor : nil,
                    switchAgent: switchAgent
                ) { await model.refresh() }
                    .overlay {
                        if model.ansi.isEmpty, !(model.isTerminal && model.terminalLocked) {
                            ProgressView("Waiting for output…").tint(.white).foregroundStyle(.white)
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
                                    .padding(.bottom, 8)
                                }
                                if !model.noticeOptions.isEmpty {
                                    ViewThatFits(in: .horizontal) {
                                        HStack { noticeButtons }
                                        VStack { noticeButtons }
                                    }
                                    .buttonStyle(.glassProminent)
                                    .padding([.horizontal, .bottom], 8)
                                    .accessibilityElement(children: .contain)
                                    .accessibilityLabel("Claude Code notice")
                                }
                            }
                        }
                    }
            }
            if let notice = model.notice ?? model.blockedHint {
                Label(notice, systemImage: "exclamationmark.triangle")
                    .font(.footnote)
                    .foregroundStyle(.orange)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(.horizontal)
                    .padding(.top, 6)
            }
            if showingChat && !model.isTerminal && (model.codexQuestionQueued || model.codexQuestionOpen || model.agent?.status == .blocked || !model.noticeOptions.isEmpty || model.commandShown) {
                Button("Continue in Terminal", systemImage: "terminal") { showingChat = false }
                    .font(.footnote).buttonStyle(.glass).padding(.top, 6)
            }
            GlassEffectContainer(spacing: 4) {
                VStack(spacing: 0) {
                    if !showingChat || model.isTerminal { KeyStrip(model: model) }
                    PromptBar(model: model)
                }
            }
        }
        .onGeometryChange(for: CGFloat.self) { $0.size.height } action: { height = $0 }
        .navigationTitle(title)
        .navigationBarTitleDisplayMode(.inline)
        .toolbar(.hidden, for: .tabBar)
        .toolbar(typingInLandscape ? .hidden : .automatic, for: .navigationBar)
        .toolbar {
            ToolbarItem(placement: .principal) { SessionTitle(title: title) }
                .sharedBackgroundVisibility(.hidden)
            ToolbarItem(placement: .primaryAction) {
                if showingChat && !model.isTerminal {
                    Button("Search conversation", systemImage: "magnifyingglass") { searchingChat.toggle() }
                } else {
                    Toggle(isOn: $model.wrapLines) { Label("Wrap lines", image: "WrapLines") }
                        .toggleStyle(.button)
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
            GitChangesView(agent: model, sent: { showingChanges = false }, review: model.gitReview)
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
        case .paste: if UIPasteboard.general.hasStrings { model.paste(UIPasteboard.general.string) }
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
    let machineLabel: String?
    @Binding var showingChat: Bool

    var body: some View {
        VStack(spacing: 6) {
            HStack(spacing: 6) {
                if let machineLabel {
                    Text(verbatim: machineLabel)
                        .font(.caption.weight(.semibold))
                        .lineLimit(1)
                        .truncationMode(.middle)
                        .layoutPriority(1)
                }
                if let agent = model.agent {
                    HStack(spacing: 6) {
                        StatusPill(state: agent.status)
                        TimelineView(.periodic(from: .now, by: 1)) { context in
                            Text(Elapsed.string(sinceMs: agent.statusSinceMs, now: context.date))
                                .font(.caption.monospacedDigit())
                                .foregroundStyle(.secondary)
                        }
                    }
                    .fixedSize()
                    .frame(maxWidth: .infinity, alignment: .leading)
                    if let pullRequest = agent.pullRequest {
                        PullRequestBadge(pullRequest: pullRequest, watch: agent.prWatch) { text in
                            model.insert(PromptShortcut(text: text))
                        }
                    }
                    if let kind = agent.kind {
                        AgentKindLabel(kind: kind, iconOnly: true).font(.caption).foregroundStyle(.secondary)
                    }
                    if let left = agent.contextLeft {
                        ContextRing(left: left)
                    }
                } else {
                    Spacer(minLength: 0)
                    if model.isTerminal {
                        AgentKindLabel(kind: "terminal").font(.caption).foregroundStyle(.secondary).fixedSize()
                        Label(model.terminalLocked ? "Locked" : "Unlocked", systemImage: model.terminalLocked ? "lock.fill" : "lock.open")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                            .fixedSize()
                    }
                }
                if !model.isTerminal {
                    // No glass backing here: next to the status pill's glass it left keyboard animations unfinished after a picker closed.
                    GlassPicker(title: "Session view", selection: $showingChat, options: [("Chat", true), ("Terminal", false)],
                                icons: [UIImage(systemName: "bubble.left.and.bubble.right"), UIImage(named: "Ghostty")])
                        .controlSize(.small)
                }
            }
            if let subagents = model.agent?.subagents, !subagents.isEmpty {
                SubagentList(subagents: subagents)
            }
            if model.link == .connecting {
                HStack(spacing: 6) {
                    ProgressView().controlSize(.mini)
                    Text("reconnecting").font(.caption).foregroundStyle(.secondary)
                }
                .frame(maxWidth: .infinity, alignment: .trailing)
            } else if let link = model.link, link != .connected {
                Text(model.linkError ?? link.label)
                    .font(.caption)
                    .foregroundStyle(.orange)
                    .lineLimit(1)
                    .frame(maxWidth: .infinity, alignment: .trailing)
            }
        }
        .padding(.horizontal)
        .padding(.vertical, 6)
    }
}

/// One line per running Claude Code subagent, laid out like Claude Code's agent panel.
struct SubagentList: View {
    let subagents: [SubagentSummary]
    @Environment(\.dynamicTypeSize) private var typeSize

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            ForEach(Array(subagents.enumerated()), id: \.offset) { _, subagent in
                TimelineView(.periodic(from: Date(timeIntervalSince1970: Double(subagent.startedAtMs / 1000)), by: 60)) { context in
                    let usage = Text(verbatim: subagent.usage(now: context.date))
                        .monospacedDigit()
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                    VStack(alignment: .leading, spacing: 0) {
                        HStack(spacing: 6) {
                            Image(systemName: "circle").font(.system(size: 7)).foregroundStyle(.secondary).accessibilityHidden(true)
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
    /// "1h 11m · ↓ 270.8k": ↓ once the subagent has called a tool, ↑ before.
    func usage(now: Date) -> String {
        let minutes = max(0, Int(now.timeIntervalSince1970) - Int(startedAtMs / 1000)) / 60
        let elapsed = switch minutes {
        case 0: "<1m"
        case ..<60: "\(minutes)m"
        default: "\(minutes / 60)h \(minutes % 60)m"
        }
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
struct SessionTitle: UIViewRepresentable {
    let title: String

    func makeUIView(context: Context) -> UILabel {
        let label = UILabel()
        label.numberOfLines = 2
        label.lineBreakMode = .byWordWrapping
        label.textAlignment = .center
        label.font = UIFontMetrics(forTextStyle: .subheadline).scaledFont(for: .systemFont(ofSize: 15, weight: .bold), maximumPointSize: 19)
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
    let neighbor: ((Int) -> AgentRoute?)?
    let switchAgent: (AgentRoute) -> Void
    let refresh: @MainActor () async -> Void

    func makeUIView(context: Context) -> GhosttyTerminalUIView {
        let view = GhosttyTerminalUIView(fontSize: fontSize)
        let control = UIRefreshControl()
        control.tintColor = .white
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
        let (gestures, perform, resized, resizeEnded) = (gestures, perform, resized, resizeEnded)
        view.onDoubleTap = gestures.doubleTap == .none ? nil : { perform(gestures.doubleTap) }
        view.onTripleTap = gestures.tripleTap == .none ? nil : { perform(gestures.tripleTap) }
        view.onPinch = gestures.pinchResizesText ? { resized($0) } : nil
        view.onPinchEnded = { resizeEnded() }
        view.onQuote = quote.map { quote in { quote($0) } }
        if gestures.swipeSwitchesAgents, let neighbor {
            let switchAgent = switchAgent
            let offset = { (swipe: TerminalSwipe) in swipe == .left ? 1 : -1 }
            view.canSwipe = { neighbor(offset($0)) != nil }
            view.onSwipe = { if let next = neighbor(offset($0)) { switchAgent(next) } }
        } else {
            view.canSwipe = nil
            view.onSwipe = nil
        }
        if !ansi.isEmpty {
            view.show(ansiSnapshot: ansi)
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

    var body: some View {
        HStack(spacing: 4) {
            ForEach(model.keyStrip, id: \.self) { key in
                Button {
                    model.tap(key)
                } label: {
                    Text(key.symbol)
                        .font(.callout.monospaced())
                        .lineLimit(1)
                        .minimumScaleFactor(0.7)
                        .frame(maxWidth: .infinity, minHeight: 22)
                }
                .buttonStyle(.glass)
                .buttonBorderShape(.roundedRectangle(radius: 8))
                .controlSize(.small)
                .accessibilityLabel(key.accessibilityName)
                .disabled(!model.accepts(key) || model.commandShown && (key == .enter || key == .ctrlEnter))
            }
        }
        .padding(.horizontal)
        .padding(.top, 8)
        .sensoryFeedback(.impact(weight: .light), trigger: model.keyTaps)
    }
}

private struct PromptBar: View {
    @Bindable var model: AgentModel
    @FocusState private var editing: Bool
    @State private var typingCommand = false
    @State private var dictationExpanded = false
    @State private var selection: TextSelection?
    @State private var pickingPhoto = false
    @State private var photos: [PhotosPickerItem] = []
    @State private var pickingFile = false
    @State private var pickingPrompt = false
    @State private var copiedAt: Date?
    @Environment(\.verticalSizeClass) private var verticalSizeClass

    private var expanded: Bool {
        editing || typingCommand || model.dictation.isActive || !model.attachments.isEmpty || (dictationExpanded && !model.draft.isEmpty)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            if let error = model.promptError {
                Text(error).font(.footnote).foregroundStyle(.red)
            }
            if model.retrying {
                Text("Unreachable, retrying…").font(.footnote).foregroundStyle(.secondary)
            }
            if let upload = model.upload {
                UploadChip(upload: upload) { model.cancelUpload() }
            }
            if let problem = model.dictation.problem {
                DictationProblemRow(problem: problem)
            }
            if !model.attachments.isEmpty {
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
            PromptInputLayout(expanded: expanded && (verticalSizeClass != .compact || model.dictation.isActive), showsStatus: model.dictation.isActive) {
                ZStack {
                    if !model.answering && !model.isTerminal {
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
                        .disabled(model.upload != nil || model.sendingPrompt)
                    }
                }
                VStack(alignment: .leading, spacing: 8) {
                    HStack(alignment: .bottom, spacing: 8) {
                        if model.isTerminal {
                            CommandField(text: $model.draft, editing: $typingCommand) {
                                Task { await model.sendPrompt() }
                            }
                        } else {
                            TextField(
                                model.answering ? "Type an answer" : "Prompt the agent", text: Binding(get: { model.draft }, set: { model.typed($0) }),
                                selection: $selection, axis: .vertical
                            )
                                .lineLimit(expanded ? 1...(verticalSizeClass == .compact ? 2 : 5) : 1...1)
                                .focused($editing)
                                .disabled(model.dictation.isActive)
                                .overlay(alignment: .trailing) {
                                    if !editing && !model.draft.isEmpty && !model.dictation.isActive {
                                        Color.clear
                                            .contentShape(Rectangle())
                                            .onTapGesture { editing = true }
                                            .onLongPressGesture {
                                                UIPasteboard.general.string = model.draft
                                                copiedAt = .now
                                            }
                                            .accessibilityHidden(true)
                                    }
                                    if copiedAt != nil {
                                        CopyIcon(label: "Copy prompt", copiedAt: $copiedAt)
                                            .allowsHitTesting(false)
                                    }
                                }
                        }
                        if expanded && !model.draft.isEmpty {
                            Button {
                                if model.isTerminal {
                                    model.draft = ""
                                } else {
                                    model.typed("")
                                }
                            } label: {
                                Image(systemName: "xmark.circle.fill")
                                    .foregroundStyle(.secondary)
                                    .frame(width: 36, height: 36)
                                    .contentShape(Rectangle())
                            }
                            .buttonStyle(.plain)
                            .disabled(model.dictation.isActive || model.sendingPrompt)
                            .accessibilityLabel("Clear text")
                        }
                    }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(.horizontal, 12)
                .padding(.vertical, 8)
                .glassEffect(.regular.interactive(), in: .rect(cornerRadius: 18))
                ZStack {
                    if model.dictation.isActive {
                        DictationBar(dictation: model.dictation)
                            .frame(maxWidth: .infinity)
                    }
                }
                HStack(spacing: 8) {
                    if !model.isTerminal && !model.dictation.isActive {
                        DictationButton(dictation: model.dictation, disabled: model.sendingPrompt) {
                            dictationExpanded = model.startDictation(at: editing ? selection : nil) != nil
                            editing = false
                        }
                    }
                    if editing || typingCommand {
                        Button {
                            editing = false
                            typingCommand = false
                        } label: {
                            Image(systemName: "keyboard.chevron.compact.down")
                                .font(.system(size: 20))
                                .frame(width: 28, height: 28)
                        }
                        .buttonStyle(.glass)
                        .buttonBorderShape(.circle)
                        .controlSize(.small)
                        .accessibilityLabel("Hide keyboard")
                    }
                    Button {
                        if model.retrying {
                            model.stopRetry()
                            return
                        }
                        if !model.keepsKeyboard {
                            editing = false
                            typingCommand = false
                        }
                        Task { await model.sendPrompt() }
                    } label: {
                        if model.sendingPrompt {
                            ProgressView().frame(width: 28, height: 28)
                        } else {
                            Image(systemName: "paperplane.fill").frame(width: 28, height: 28)
                        }
                    }
                    .buttonStyle(.glassProminent)
                    .buttonBorderShape(.circle)
                    .controlSize(.small)
                    .disabled(!model.canSendPrompt && !model.retrying)
                    .accessibilityLabel(model.retrying ? "Stop retrying" : model.isTerminal ? "Run command" : model.answering ? "Send answer" : "Send")
                }
            }
        }
        .padding(.horizontal)
        .padding(.vertical, 8)
        .sheet(isPresented: $pickingPrompt) {
            NavigationStack {
                PromptShortcutsView { shortcut in
                    model.insert(shortcut)
                    editing = true
                }
            }
        }
        .photosPicker(
            isPresented: $pickingPhoto, selection: $photos, maxSelectionCount: max(model.attachmentSlots, 1),
            matching: .images
        )
        .onChange(of: photos) { _, items in
            guard !items.isEmpty else { return }
            photos = []
            let now = Date.now
            model.attach(items.enumerated().map { offset, item in
                PendingAttachment(name: Attachment.photoName(at: now, index: offset + 1)) { _ in
                    guard let data = try await item.loadTransferable(type: Data.self) else { throw AttachmentError.unreadablePhoto }
                    return try Attachment.jpeg(from: data)
                }
            })
        }
        .fileImporter(isPresented: $pickingFile, allowedContentTypes: [.item], allowsMultipleSelection: true) { result in
            switch result {
            case .success(let urls):
                model.attach(urls.map { url in
                    PendingAttachment(name: Attachment.suggestedName(url.lastPathComponent)) { limit in
                        try Attachment.read(url, limit: limit)
                    }
                })
            case .failure(let error):
                model.attachFailed(error)
            }
        }
        .onChange(of: model.quotes) {
            selection = TextSelection(insertionPoint: model.draft.endIndex)
            editing = true
        }
        .onChange(of: model.draft.isEmpty) { _, empty in
            if empty && !model.dictation.isActive { dictationExpanded = false }
        }
        .onChange(of: model.dictation.isActive) { _, active in
            if !active && model.draft.isEmpty { dictationExpanded = false }
        }
        .onDisappear { dictationExpanded = false }
    }
}

/// A shell command goes out exactly as typed: no autocorrection, capitals, smart quotes or
/// dashes, which would change it (`--force` into a dash), and one line.
private struct CommandField: UIViewRepresentable {
    @Binding var text: String
    @Binding var editing: Bool
    let submit: () -> Void

    func makeUIView(context: Context) -> UITextField {
        let field = UITextField()
        field.placeholder = "Command"
        field.autocorrectionType = .no
        field.autocapitalizationType = .none
        field.spellCheckingType = .no
        field.smartQuotesType = .no
        field.smartDashesType = .no
        field.smartInsertDeleteType = .no
        field.inlinePredictionType = .no
        field.keyboardType = .asciiCapable
        field.returnKeyType = .go
        field.font = UIFontMetrics(forTextStyle: .body).scaledFont(for: .monospacedSystemFont(ofSize: UIFont.labelFontSize, weight: .regular))
        field.adjustsFontForContentSizeCategory = true
        field.setContentHuggingPriority(.required, for: .vertical)
        let coordinator = context.coordinator
        field.delegate = coordinator
        field.addAction(UIAction { [weak field] _ in coordinator.parent.text = field?.text ?? "" }, for: .editingChanged)
        return field
    }

    func updateUIView(_ field: UITextField, context: Context) {
        context.coordinator.parent = self
        if field.text != text { field.text = text }
        if !editing, field.isFirstResponder { field.resignFirstResponder() }
    }

    func makeCoordinator() -> Coordinator { Coordinator(parent: self) }

    @MainActor
    final class Coordinator: NSObject, UITextFieldDelegate {
        var parent: CommandField

        init(parent: CommandField) {
            self.parent = parent
        }

        func textFieldDidBeginEditing(_ textField: UITextField) {
            parent.editing = true
        }

        func textFieldDidEndEditing(_ textField: UITextField) {
            parent.editing = false
        }

        func textFieldShouldReturn(_ textField: UITextField) -> Bool {
            parent.submit()
            return false
        }
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
            Image(systemName: dictation.isActive ? "mic.fill" : "mic")
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
        .buttonBorderShape(.circle)
        .controlSize(.small)
        .disabled(disabled)
        .accessibilityLabel(dictation.isActive ? "Stop dictation" : "Dictate in \(dictation.language.label)")
    }
}

struct DictationBar: View {
    let dictation: DictationModel

    var body: some View {
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
                Text(dictation.phase == .finishing ? "Finishing…" : "Starting…").font(.caption)
            }
            if dictation.phase != .listening { Spacer(minLength: 0) }
            Button(dictation.language.code) { dictation.select(dictation.language.other) }
                .font(.caption.monospaced().weight(.semibold))
                .buttonStyle(.plain)
                .controlSize(.mini)
                .disabled(dictation.phase == .finishing)
                .accessibilityLabel("Dictation language, \(dictation.language.label)")
                .accessibilityHint("Switches to \(dictation.language.other.label)")
            Button(action: dictation.stop) {
                Image(systemName: "stop.circle.fill").font(.title3).foregroundStyle(.red)
            }
            .buttonStyle(.plain)
            .disabled(dictation.phase == .finishing)
            .accessibilityLabel("Stop dictation")
        }
        .padding(.leading, 12)
        .padding(.trailing, 6)
        .padding(.vertical, 4)
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
