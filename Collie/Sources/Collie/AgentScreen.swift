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
    @Environment(\.dismiss) private var dismiss
    @Environment(\.scenePhase) private var scenePhase

    init(
        core: any AgentCore, route: AgentRoute, approvals: ApprovalsModel? = nil, follows: FollowModel? = nil,
        machineLabel: String? = nil, showsMachine: Bool = false, neighbor: ((Int) -> AgentRoute?)? = nil,
        switchAgent: @escaping (AgentRoute) -> Void = { _ in }
    ) {
        _model = State(initialValue: AgentModel(core: core, route: route, machineLabel: machineLabel))
        self.approvals = approvals
        self.follows = follows
        self.machineLabel = machineLabel
        self.showsMachine = showsMachine
        self.neighbor = neighbor
        self.switchAgent = switchAgent
    }

    // Switching agents drops these, as Back does; text loaded from the Mac's input box loads again.
    private var holdsUnsent: Bool {
        let typed = !model.draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty && model.draft != model.macDraft
        return typed || !model.attachments.isEmpty || model.upload != nil
    }

    private var blocked: BlockedInput? {
        approvals?.blockedInput(machineId: model.route.machineId, terminalId: model.route.terminalId)
    }

    var body: some View {
        VStack(spacing: 0) {
            AgentHeader(model: model, machineLabel: showsMachine ? machineLabel : nil)
            if let notice = model.paneNotice {
                PaneCard(model: model, notice: notice)
            }
            if let approvals {
                ForEach(approvals.items(machineId: model.route.machineId, terminalId: model.route.terminalId)) { item in
                    ApprovalCard(model: approvals, item: item)
                        .padding(.horizontal)
                        .background(Color.red.opacity(0.08))
                }
                if let notice = approvals.notice {
                    Text(notice).font(.footnote).padding(.horizontal).frame(maxWidth: .infinity, alignment: .leading)
                }
            }
            AgentTerminal(
                ansi: model.ansi, wraps: model.wrapLines, fontSize: model.fontSize, gestures: model.gestures,
                perform: perform, resized: { model.fontSize = $0 }, neighbor: holdsUnsent ? nil : neighbor,
                switchAgent: switchAgent
            ) { await model.refresh() }
                .overlay {
                    if model.ansi.isEmpty, !(model.isTerminal && model.terminalLocked) {
                        ProgressView("Waiting for output…").tint(.white).foregroundStyle(.white)
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
            KeyStrip(model: model)
            PromptBar(model: model)
        }
        .navigationTitle(model.agent?.displayTitle ?? model.terminal?.displayTitle ?? "Agent")
        .navigationBarTitleDisplayMode(.inline)
        .toolbar(.hidden, for: .tabBar)
        .toolbar {
            ToolbarItem(placement: .primaryAction) {
                Toggle(isOn: $model.wrapLines) {
                    // SF Symbols has no wrap-text glyph; WrapLines is a drawn template image.
                    Label("Wrap lines", image: "WrapLines")
                }
                .toggleStyle(.button)
            }
            ToolbarItem(placement: .primaryAction) {
                Menu("More", systemImage: "ellipsis") {
                    if !model.isTerminal {
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
                    }
                    Divider()
                    Button("Close pane", systemImage: "xmark.square", role: .destructive) {
                        model.close.begin(.pane)
                    }
                    Button("Close workspace", systemImage: "xmark.rectangle.portrait", role: .destructive) {
                        if let workspaceId = model.agent?.workspaceId ?? model.terminal?.workspaceId {
                            model.close.begin(.workspace(id: workspaceId))
                        }
                    }
                    .disabled(model.agent == nil && model.terminal == nil)
                }
            }
        }
        .onChange(of: blocked, initial: true) { _, blocked in model.blocked = blocked }
        .closeConfirmation($model.close) { await model.performClose() }
        .onChange(of: model.closed) { _, closed in
            if closed { dismiss() }
        }
        .task { await model.run() }
        .onAppear { model.reloadGestures() }
        .onChange(of: scenePhase) { _, phase in model.dictation.scenePhaseChanged(to: phase) }
        .onDisappear {
            model.cancelUpload()
            model.dictation.cancel()
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
    /// Presents both steps of `close` once it begins; `perform` only runs from the second.
    func closeConfirmation(_ close: Binding<CloseConfirmation>, perform: @escaping @MainActor () async -> Void) -> some View {
        modifier(CloseDialogs(close: close, perform: perform))
    }
}

private struct CloseDialogs: ViewModifier {
    @Binding var close: CloseConfirmation
    let perform: @MainActor () async -> Void
    @State private var confirmingFirst = false
    @State private var confirmingSecond = false

    func body(content: Content) -> some View {
        content
            .onChange(of: close.step) { _, step in
                if case .first = step { confirmingFirst = true }
            }
            .onChange(of: confirmingFirst) { _, shown in
                if !shown, case .first = close.step { close.cancel() }
            }
            .confirmationDialog(title, isPresented: $confirmingFirst, titleVisibility: .visible) {
                Button(action, role: .destructive) {
                    close.advance()
                    confirmingSecond = true
                }
                Button("Cancel", role: .cancel) { close.cancel() }
            } message: {
                Text(message)
            }
            .alert("Are you sure?", isPresented: $confirmingSecond) {
                Button(action, role: .destructive) {
                    Task { await perform() }
                }
                Button("Cancel", role: .cancel) { close.cancel() }
            } message: {
                Text("\(message) This cannot be undone.")
            }
    }

    private var title: String {
        close.target == .pane ? "Close this pane?" : "Close this workspace?"
    }

    private var action: String {
        close.target == .pane ? "Close pane" : "Close workspace"
    }

    private var message: String {
        close.target == .pane
            ? "The agent and its shell on the machine are ended."
            : "Every agent and shell in the workspace on the machine is ended."
    }
}

private struct AgentHeader: View {
    let model: AgentModel
    let machineLabel: String?

    var body: some View {
        HStack(spacing: 8) {
            if let machineLabel {
                Text(verbatim: machineLabel).font(.caption.weight(.semibold)).lineLimit(1)
            }
            if let agent = model.agent {
                StatusPill(state: agent.status)
                TimelineView(.periodic(from: .now, by: 1)) { context in
                    Text(Elapsed.string(sinceMs: agent.statusSinceMs, now: context.date))
                        .font(.caption.monospacedDigit())
                        .foregroundStyle(.secondary)
                }
                if let kind = agent.kind {
                    AgentKindLabel(kind: kind).font(.caption).foregroundStyle(.secondary)
                }
                if let left = agent.contextLeft {
                    HStack(spacing: 4) {
                        ContextRing(left: left)
                        Text("\(left)% context").font(.caption.monospacedDigit()).foregroundStyle(.secondary)
                    }
                    .accessibilityElement(children: .ignore)
                    .accessibilityLabel("Context \(left)% left")
                }
            } else if model.isTerminal {
                AgentKindLabel(kind: "terminal").font(.caption).foregroundStyle(.secondary)
                Label(model.terminalLocked ? "Locked" : "Unlocked", systemImage: model.terminalLocked ? "lock.fill" : "lock.open")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            Spacer()
            if model.link == .connecting {
                ProgressView().controlSize(.mini)
                Text("reconnecting").font(.caption).foregroundStyle(.secondary)
            } else if let link = model.link, link != .connected {
                Text(model.linkError ?? link.label)
                    .font(.caption)
                    .foregroundStyle(.orange)
                    .lineLimit(1)
            }
        }
        .padding(.horizontal)
        .padding(.vertical, 6)
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
        let (gestures, perform, resized) = (gestures, perform, resized)
        view.onDoubleTap = gestures.doubleTap == .none ? nil : { perform(gestures.doubleTap) }
        view.onTripleTap = gestures.tripleTap == .none ? nil : { perform(gestures.tripleTap) }
        view.onPinch = gestures.pinchResizesText ? { resized($0) } : nil
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
                .buttonStyle(.borderedProminent)
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
            ForEach(model.isTerminal ? AgentKey.terminalStrip : AgentKey.strip, id: \.self) { key in
                Button {
                    model.tap(key)
                } label: {
                    Text(key.symbol)
                        .font(.callout.monospaced())
                        .lineLimit(1)
                        .minimumScaleFactor(0.7)
                        .frame(maxWidth: .infinity, minHeight: 34)
                        .background(.fill.tertiary, in: RoundedRectangle(cornerRadius: 8))
                        .contentShape(RoundedRectangle(cornerRadius: 8))
                }
                .buttonStyle(.plain)
                .accessibilityLabel(key.accessibilityName)
            }
        }
        .disabled(!model.acceptsKeys)
        .padding(.horizontal)
        .padding(.top, 8)
        .sensoryFeedback(.impact(weight: .light), trigger: model.keyTaps)
    }
}

private struct PromptBar: View {
    @Bindable var model: AgentModel
    @FocusState private var editing: Bool
    @State private var typingCommand = false
    @State private var pickingPhoto = false
    @State private var photos: [PhotosPickerItem] = []
    @State private var pickingFile = false

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            if let error = model.promptError {
                Text(error).font(.footnote).foregroundStyle(.red)
            }
            if let upload = model.upload {
                UploadChip(upload: upload) { model.cancelUpload() }
            }
            if let problem = model.dictation.problem {
                DictationProblemRow(problem: problem)
            }
            if model.dictation.isActive {
                DictationBar(dictation: model.dictation)
            }
            if !model.attachments.isEmpty {
                ScrollView(.horizontal) {
                    HStack(spacing: 6) {
                        ForEach(model.attachments) { file in
                            AttachmentPill(file: file) { model.remove(file) }
                        }
                    }
                }
                .disabled(model.sendingPrompt)
                .scrollIndicators(.hidden)
            }
            HStack(alignment: .bottom, spacing: 8) {
                if !model.answering && !model.isTerminal {
                    Menu {
                        Button("Photo Library", systemImage: "photo.on.rectangle") { pickingPhoto = true }
                        Button("Files", systemImage: "folder") { pickingFile = true }
                    } label: {
                        Image(systemName: "paperclip")
                            .font(.system(size: 20))
                            .frame(width: 32, height: 36)
                    }
                    .disabled(model.upload != nil || model.attachmentSlots <= 0)
                    .accessibilityLabel("Attach")
                }
                if model.isTerminal {
                    CommandField(text: $model.draft, editing: $typingCommand) {
                        Task { await model.sendPrompt() }
                    }
                    .frame(minHeight: 36)
                    .padding(.horizontal, 12)
                    .background(.fill.tertiary, in: RoundedRectangle(cornerRadius: 18))
                } else {
                    TextField(model.answering ? "Type an answer" : "Prompt the agent", text: $model.draft, axis: .vertical)
                        .lineLimit(1...6)
                        .padding(.horizontal, 12)
                        .padding(.vertical, 8)
                        .background(.fill.tertiary, in: RoundedRectangle(cornerRadius: 18))
                        .focused($editing)
                        .disabled(model.dictation.isActive)
                    DictationButton(model: model) { editing = false }
                }
                if editing || typingCommand {
                    Button {
                        editing = false
                        typingCommand = false
                    } label: {
                        Image(systemName: "keyboard.chevron.compact.down")
                            .font(.system(size: 20))
                            .frame(width: 32, height: 36)
                    }
                    .accessibilityLabel("Hide keyboard")
                }
                Button {
                    if !model.keepsKeyboard {
                        editing = false
                        typingCommand = false
                    }
                    Task { await model.sendPrompt() }
                } label: {
                    if model.sendingPrompt {
                        ProgressView().frame(width: 32, height: 32)
                    } else {
                        Image(systemName: "arrow.up.circle.fill").font(.system(size: 32))
                    }
                }
                .disabled(!model.canSendPrompt)
                .accessibilityLabel(model.isTerminal ? "Run command" : model.answering ? "Send answer" : "Send prompt")
            }
        }
        .padding(.horizontal)
        .padding(.vertical, 8)
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

private struct DictationButton: View {
    let model: AgentModel
    let starting: () -> Void

    var body: some View {
        let dictation = model.dictation
        Menu {
            Picker(
                "Dictation language",
                selection: Binding(get: { dictation.language }, set: { dictation.select($0) })
            ) {
                ForEach(DictationLanguage.allCases, id: \.self) { Text($0.label).tag($0) }
            }
        } label: {
            Image(systemName: dictation.isActive ? "mic.fill" : "mic")
                .font(.system(size: 20))
                .frame(width: 32, height: 36)
        } primaryAction: {
            if dictation.isActive {
                dictation.stop()
            } else {
                starting()
                model.startDictation()
            }
        }
        .disabled(model.sendingPrompt)
        .accessibilityLabel(dictation.isActive ? "Stop dictation" : "Dictate in \(dictation.language.label)")
    }
}

private struct DictationBar: View {
    let dictation: DictationModel

    var body: some View {
        HStack(spacing: 8) {
            switch dictation.phase {
            case .preparing(let download?):
                Text("Downloading speech model").font(.caption).lineLimit(1)
                ProgressView(value: download).frame(width: 60)
            case .listening:
                Text("Dictating…").font(.caption.weight(.semibold))
                LevelMeter(level: dictation.level)
            case .idle, .preparing(nil), .finishing:
                ProgressView().controlSize(.mini)
                Text(dictation.phase == .finishing ? "Finishing…" : "Starting…").font(.caption)
            }
            Spacer(minLength: 0)
            Button(dictation.language.code) { dictation.select(dictation.language.other) }
                .font(.caption.monospaced().weight(.semibold))
                .buttonStyle(.bordered)
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
        .background(.fill.tertiary, in: Capsule())
    }
}

private struct LevelMeter: View {
    let level: Float

    var body: some View {
        HStack(spacing: 2) {
            ForEach(0..<8, id: \.self) { index in
                Capsule()
                    .fill(Float(index) < level * 8 ? Color.red : Color.secondary.opacity(0.3))
                    .frame(width: 3, height: 12)
            }
        }
        .animation(.linear(duration: 0.1), value: level)
        .accessibilityHidden(true)
    }
}

private struct DictationProblemRow: View {
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

private struct AttachmentPill: View {
    let file: AttachedFile
    let remove: () -> Void

    var body: some View {
        HStack(spacing: 6) {
            if let thumbnail = file.thumbnail {
                Image(uiImage: thumbnail)
                    .resizable()
                    .scaledToFill()
                    .frame(width: 22, height: 22)
                    .clipShape(RoundedRectangle(cornerRadius: 5))
            } else {
                Image(systemName: file.symbol).font(.caption).foregroundStyle(.secondary).frame(width: 22, height: 22)
            }
            Text(shortName)
                .font(.caption)
                .lineLimit(1)
                .accessibilityLabel(file.name)
            Button(action: remove) {
                Image(systemName: "xmark.circle.fill").foregroundStyle(.secondary)
            }
            .buttonStyle(.plain)
            .accessibilityLabel("Remove \(file.name)")
        }
        .padding(.leading, 4)
        .padding(.trailing, 8)
        .padding(.vertical, 4)
        .background(.fill.tertiary, in: Capsule())
    }

    /// A horizontal ScrollView proposes no width, so `truncationMode` would never kick in.
    private var shortName: String {
        file.name.count <= 22 ? file.name : "\(file.name.prefix(10))…\(file.name.suffix(10))"
    }
}

private struct UploadChip: View {
    let upload: AttachmentUpload
    let cancel: () -> Void

    var body: some View {
        HStack(spacing: 8) {
            Image(systemName: "paperclip").font(.caption).foregroundStyle(.secondary)
            Text(upload.count > 1 ? "\(upload.name) (\(upload.index) of \(upload.count))" : upload.name)
                .font(.caption)
                .lineLimit(1)
                .truncationMode(.middle)
            ProgressView(value: upload.fraction ?? 0).frame(width: 72)
            Button(action: cancel) {
                Image(systemName: "xmark.circle.fill").foregroundStyle(.secondary)
            }
            .buttonStyle(.plain)
            .accessibilityLabel("Cancel upload")
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 6)
        .background(.fill.tertiary, in: Capsule())
    }
}
