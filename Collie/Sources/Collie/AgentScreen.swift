import CollieCore
import GhosttyTerminal
import PhotosUI
import SwiftUI
import UIKit

struct AgentScreen: View {
    @State private var model: AgentModel
    let approvals: ApprovalsModel?
    let follows: FollowModel?
    @Environment(\.dismiss) private var dismiss

    init(core: any AgentCore, route: AgentRoute, approvals: ApprovalsModel? = nil, follows: FollowModel? = nil) {
        _model = State(initialValue: AgentModel(core: core, route: route))
        self.approvals = approvals
        self.follows = follows
    }

    private var blocked: BlockedInput? {
        approvals?.blockedInput(machineId: model.route.machineId, terminalId: model.route.terminalId)
    }

    var body: some View {
        VStack(spacing: 0) {
            AgentHeader(model: model)
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
            AgentTerminal(ansi: model.ansi, wraps: model.wrapLines) { await model.refresh() }
                .overlay {
                    if model.ansi.isEmpty {
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
        .navigationTitle(model.agent?.displayTitle ?? "Agent")
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
                    Button("Refresh", systemImage: "arrow.clockwise") {
                        Task { await model.refresh() }
                    }
                    .disabled(model.refreshing)
                    Button("Focus on Mac", systemImage: "macwindow") {
                        Task { await model.focus() }
                    }
                    if let follows {
                        FollowMenuItem(follows: follows, route: model.route)
                    }
                    Divider()
                    Button("Close pane", systemImage: "xmark.square", role: .destructive) {
                        model.close.begin(.pane)
                    }
                    Button("Close workspace", systemImage: "xmark.rectangle.portrait", role: .destructive) {
                        if let workspaceId = model.agent?.workspaceId {
                            model.close.begin(.workspace(id: workspaceId))
                        }
                    }
                    .disabled(model.agent == nil)
                }
            }
        }
        .onChange(of: blocked, initial: true) { _, blocked in model.blocked = blocked }
        .closeConfirmation($model.close) { await model.performClose() }
        .onChange(of: model.closed) { _, closed in
            if closed { dismiss() }
        }
        .task { await model.run() }
        .onDisappear { model.cancelUpload() }
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
            ? "The agent and its shell on the Mac are ended."
            : "Every agent and shell in the workspace on the Mac is ended."
    }
}

private struct AgentHeader: View {
    let model: AgentModel

    var body: some View {
        HStack(spacing: 8) {
            if let agent = model.agent {
                StatusPill(state: agent.status)
                TimelineView(.periodic(from: .now, by: 1)) { context in
                    Text(Elapsed.string(sinceMs: agent.statusSinceMs, now: context.date))
                        .font(.caption.monospacedDigit())
                        .foregroundStyle(.secondary)
                }
                if let kind = agent.kind {
                    Text(kind).font(.caption).foregroundStyle(.secondary)
                }
            } else {
                Text("Agent not in the flock").font(.caption).foregroundStyle(.secondary)
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
    let refresh: @MainActor () async -> Void

    func makeUIView(context: Context) -> GhosttyTerminalUIView {
        let view = GhosttyTerminalUIView(fontSize: 11)
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

private struct KeyStrip: View {
    let model: AgentModel

    var body: some View {
        HStack(spacing: 4) {
            ForEach(AgentKey.strip, id: \.self) { key in
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
                if !model.answering {
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
                TextField(model.answering ? "Type an answer" : "Prompt the agent", text: $model.draft, axis: .vertical)
                    .lineLimit(1...6)
                    .padding(.horizontal, 12)
                    .padding(.vertical, 8)
                    .background(.fill.tertiary, in: RoundedRectangle(cornerRadius: 18))
                    .focused($editing)
                if editing {
                    Button {
                        editing = false
                    } label: {
                        Image(systemName: "keyboard.chevron.compact.down")
                            .font(.system(size: 20))
                            .frame(width: 32, height: 36)
                    }
                    .accessibilityLabel("Hide keyboard")
                }
                Button {
                    if !model.keepsKeyboard { editing = false }
                    Task { await model.sendPrompt() }
                } label: {
                    if model.sendingPrompt {
                        ProgressView().frame(width: 32, height: 32)
                    } else {
                        Image(systemName: "arrow.up.circle.fill").font(.system(size: 32))
                    }
                }
                .disabled(!model.canSendPrompt)
                .accessibilityLabel(model.answering ? "Send answer" : "Send prompt")
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
