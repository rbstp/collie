import CollieCore
import SwiftUI

struct NewTaskSheet: View {
    let machines: [Machine]
    let onStarted: (AgentRoute) -> Void
    @State private var model: NewTaskModel
    @State private var startTask: Task<Void, Never>?
    @State private var pickingPhoto = false
    @State private var pickingFile = false
    @State private var pickingPrompt = false
    @State private var dictationExpanded = false
    @State private var dictationShown = false
    @State private var selection: TextSelection?
    @FocusState private var editingPrompt: Bool
    @Environment(\.dismiss) private var dismiss
    @Environment(\.scenePhase) private var scenePhase
    @Environment(\.verticalSizeClass) private var verticalSizeClass

    private var expandedPrompt: Bool {
        editingPrompt || model.dictation.isActive || dictationShown || !model.attachments.isEmpty || (dictationExpanded && !model.prompt.isEmpty)
    }

    init(
        core: any AgentCore, machines: [Machine], preferredMachineId: String?,
        onStarted: @escaping (AgentRoute) -> Void
    ) {
        self.init(
            model: NewTaskModel(core: core, machines: machines, preferredMachineId: preferredMachineId),
            machines: machines, onStarted: onStarted
        )
    }

    init(model: NewTaskModel, machines: [Machine], onStarted: @escaping (AgentRoute) -> Void) {
        self.machines = machines
        self.onStarted = onStarted
        _model = State(initialValue: model)
    }

    private func cancel() {
        model.cancel()
        startTask?.cancel()
    }

    var body: some View {
        NavigationStack {
            Form {
                if machines.count > 1 {
                    Picker("Machine", selection: $model.machineId) {
                        ForEach(machines, id: \.id) { machine in
                            Text(machine.label).tag(Optional(machine.id))
                        }
                    }
                    .tint(.primary)
                }

                Picker("Location", selection: $model.location) {
                    ForEach(NewTaskModel.Location.allCases, id: \.self) { location in
                        Text(location.rawValue).tag(location)
                    }
                }
                .tint(.primary)

                if model.location == .folder {
                    Section {
                        Toggle("New folder", isOn: $model.newFolder)
                        if model.newFolder {
                            if model.base == nil, let roots = model.options?.roots, roots.count > 1 {
                                Picker("In", selection: Binding(get: { model.newFolderParent }, set: { model.newFolderRoot = $0 })) {
                                    ForEach(roots, id: \.self) { Text($0).tag(Optional($0)) }
                                }
                                .tint(.primary)
                            }
                            TextField("Folder name", text: $model.folderName)
                                .textInputAutocapitalization(.never)
                                .autocorrectionDisabled()
                                .font(.body.monospaced())
                        } else {
                            folderPicker($model.cwd, completions: model.completions)
                        }
                    } header: {
                        Text("Folder")
                    } footer: {
                        VStack(alignment: .leading, spacing: 4) {
                            if model.newFolder, let parent = model.newFolderParent {
                                let name = model.folderName.trimmingCharacters(in: .whitespacesAndNewlines)
                                Text("Creates \(parent)/\(name.isEmpty ? "name" : name) (empty) and starts the agent there.")
                            }
                            if let error = model.optionsError {
                                Text(error).foregroundStyle(.red)
                            }
                            if let error = model.foldersError {
                                Text(error).foregroundStyle(.red)
                            }
                        }
                    }
                } else if model.location == .clone {
                    Section {
                        if model.base == nil, let roots = model.options?.roots, roots.count > 1 {
                            Picker("In", selection: Binding(get: { model.newFolderParent }, set: { model.newFolderRoot = $0 })) {
                                ForEach(roots, id: \.self) { Text($0).tag(Optional($0)) }
                            }
                            .tint(.primary)
                        }
                        TextField("owner/repo or URL", text: $model.repository)
                            .textInputAutocapitalization(.never)
                            .autocorrectionDisabled()
                            .keyboardType(.URL)
                            .font(.body.monospaced())
                    } header: {
                        Text("Repository")
                    } footer: {
                        VStack(alignment: .leading, spacing: 4) {
                            if let parent = model.newFolderParent {
                                Text("Clones into \(parent)/\(model.cloneFolder ?? "repo") and starts the agent there.")
                            }
                            if let error = model.optionsError {
                                Text(error).foregroundStyle(.red)
                            }
                        }
                    }
                } else if model.location == .resume {
                    Section {
                        folderPicker($model.cwd, completions: model.completions)
                    } header: {
                        Text("Folder")
                    } footer: {
                        VStack(alignment: .leading, spacing: 4) {
                            if let error = model.optionsError {
                                Text(error).foregroundStyle(.red)
                            }
                            if let error = model.foldersError {
                                Text(error).foregroundStyle(.red)
                            }
                        }
                    }
                    Section("Session") {
                        if model.sessions == nil, model.sessionsError == nil, model.folder != nil {
                            ProgressView("Finding sessions…")
                        }
                        if let sessions = model.sessions {
                            if sessions.isEmpty {
                                Text("No Claude Code or Codex session in this folder.").foregroundStyle(.secondary)
                            }
                            ForEach(sessions, id: \.sessionId) { session in
                                Button {
                                    model.session = session
                                } label: {
                                    HStack {
                                        VStack(alignment: .leading, spacing: 2) {
                                            Text(session.firstPrompt.isEmpty ? "Prompt not shown" : session.firstPrompt).lineLimit(2)
                                            HStack {
                                                AgentKindLabel(kind: session.agent)
                                                Text(Date(timeIntervalSince1970: Double(session.modifiedMs) / 1000), format: .relative(presentation: .named))
                                            }
                                            .font(.footnote)
                                            .foregroundStyle(.secondary)
                                        }
                                        Spacer()
                                        if model.session?.sessionId == session.sessionId { Image(systemName: "checkmark") }
                                    }
                                }
                                .foregroundStyle(.primary)
                            }
                        }
                        if let error = model.sessionsError { Text(error).foregroundStyle(.red) }
                    }
                } else {
                    Section("Git worktree") {
                        Picker("Action", selection: $model.worktreeAction) {
                            ForEach(NewTaskModel.WorktreeAction.allCases, id: \.self) { action in
                                Text(action.rawValue).tag(action)
                            }
                        }
                        .tint(.primary)
                        folderPicker($model.source, completions: model.sourceCompletions)
                        if model.worktrees == nil, model.worktreesError == nil, model.sourceFolder != nil {
                            ProgressView("Checking repository…")
                        }
                        if let error = model.optionsError { Text(error).foregroundStyle(.red) }
                        if let error = model.foldersError { Text(error).foregroundStyle(.red) }
                        if let listed = model.worktrees {
                            Text("Repository: \(listed.source)").font(.footnote.monospaced())
                            if model.worktreeAction == .create {
                                TextField("Branch name (optional)", text: $model.branch)
                                    .textInputAutocapitalization(.never)
                                    .autocorrectionDisabled()
                                if let path = model.createPath {
                                    Text("Create \(model.createBranch) at \(path)")
                                        .font(.footnote)
                                }
                            } else if model.worktreeAction == .pullRequest {
                                TextField("Pull request number", text: $model.pullRequestNumber)
                                    .keyboardType(.numberPad)
                                if let number = model.pullRequest {
                                    Text("Check out the branch of #\(number) under \(listed.source)/.worktree/")
                                        .font(.footnote)
                                }
                            } else {
                                ForEach(listed.worktrees, id: \.path) { checkout in
                                    Button {
                                        model.checkoutPath = checkout.path
                                    } label: {
                                        HStack {
                                            VStack(alignment: .leading) {
                                                Text(checkout.branch ?? "Detached")
                                                Text(checkout.path).font(.footnote.monospaced())
                                            }
                                            Spacer()
                                            if checkout.open { Text("Open").font(.footnote) }
                                            if model.checkoutPath == checkout.path { Image(systemName: "checkmark") }
                                        }
                                    }
                                    .foregroundStyle(.primary)
                                }
                            }
                        }
                        if let error = model.worktreesError { Text(error).foregroundStyle(.red) }
                    }
                }

                if model.location != .resume {
                    Section {
                        if let agents = model.options?.agents {
                            Picker("Agent", selection: $model.agent) {
                                ForEach(agents, id: \.self) { AgentKindLabel(kind: $0).tag($0) }
                            }
                            .tint(.primary)
                            if model.agent == "claude" {
                                Toggle("Ultracode", isOn: $model.ultracode)
                            }
                        } else if model.optionsError == nil {
                            ProgressView()
                        }
                    } header: {
                        Text("Agent")
                    } footer: {
                        if !model.agent.isEmpty {
                            Text("Use the session's terminal buttons or type an answer to respond to approval prompts.")
                        }
                    }
                }

                Section(model.location == .resume ? "Prompt (optional)" : "Prompt") {
                    if let error = model.attachmentError {
                        Text(error).font(.footnote).foregroundStyle(.red)
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
                        .scrollIndicators(.hidden)
                    }
                    GlassEffectContainer(spacing: 4) {
                        PromptInputLayout(expanded: expandedPrompt && (verticalSizeClass != .compact || model.dictation.isActive || dictationShown)) {
                            ZStack {
                                AttachmentMenu(
                                    savedPromptsEnabled: !model.dictation.isActive,
                                    filesEnabled: model.attachmentSlots > 0
                                ) { choice in
                                    switch choice {
                                    case .savedPrompts: pickingPrompt = true
                                    case .files: pickingFile = true
                                    case .photos: pickingPhoto = true
                                    }
                                }
                                .disabled(model.upload != nil || model.phase != .editing)
                            }
                            HStack(alignment: .bottom, spacing: 8) {
                                TextField("What should the agent do?", text: $model.prompt, selection: $selection, axis: .vertical)
                                    .lineLimit(expandedPrompt ? 1...(verticalSizeClass == .compact ? 2 : 5) : 1...1)
                                    .focused($editingPrompt)
                                    .disabled(model.dictation.isActive)
                                if expandedPrompt && !model.prompt.isEmpty {
                                    Button {
                                        model.prompt = ""
                                    } label: {
                                        Image(systemName: "xmark.circle.fill")
                                            .foregroundStyle(.secondary)
                                            .frame(width: 36, height: 36)
                                            .contentShape(Rectangle())
                                    }
                                    .buttonStyle(.plain)
                                    .disabled(model.dictation.isActive || model.phase != .editing)
                                    .accessibilityLabel("Clear text")
                                }
                            }
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .padding(.horizontal, 12)
                            .padding(.vertical, 8)
                            .glassEffect(.regular.interactive(), in: .rect(cornerRadius: 18))
                            DictationControl(dictation: model.dictation, disabled: model.phase != .editing, shown: $dictationShown) {
                                dictationExpanded = model.startDictation(at: editingPrompt ? selection : nil) != nil
                                editingPrompt = false
                            }
                            HStack(spacing: 8) {
                                if editingPrompt {
                                    Button {
                                        editingPrompt = false
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
                            }
                        }
                    }
                    .id(KeepsPromptVisible.id)
                }

                Section {
                    TextField("Label (optional)", text: $model.label)
                } footer: {
                    Text("Names the new workspace in herdr.")
                }

                Section {
                    Button {
                        startTask = Task {
                            if let route = await model.start() {
                                dismiss()
                                onStarted(route)
                            }
                        }
                    } label: {
                        switch model.phase {
                        case .editing, .startedNotVisible:
                            Text("Start")
                        case .starting:
                            HStack { ProgressView(); Text("Starting the task…") }
                        case .waitingForAgent:
                            HStack { ProgressView(); Text("Waiting for the agent…") }
                        }
                    }
                    .disabled(!model.canStart)
                    if let error = model.error {
                        Label(error, systemImage: "xmark.octagon").foregroundStyle(.red)
                    }
                }
            }
            .safeAreaPadding(.bottom, editingPrompt ? 48 : 0)
            .modifier(KeepsPromptVisible(editing: editingPrompt))
            .safeAreaInset(edge: .top) {
                if let repository = model.cloning {
                    HStack(spacing: 10) {
                        ProgressView().controlSize(.small)
                        Text("Cloning \(repository)…")
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
            .navigationTitle("New task")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button(model.phase == .startedNotVisible ? "Done" : "Cancel") {
                        cancel()
                        dismiss()
                    }
                }
            }
            .task(id: model.machineId) { await model.loadOptions() }
            .task(id: model.location == .worktree ? "\(model.machineId ?? ""):\(model.worktreeAction.rawValue):\(model.base ?? ""):\(model.source)" : nil) {
                guard model.location == .worktree, model.sourceFolder != nil else {
                    return
                }
                try? await Task.sleep(for: .milliseconds(250))
                guard !Task.isCancelled else { return }
                await model.loadWorktrees()
            }
            .task(id: model.location == .resume ? "\(model.machineId ?? ""):\(model.base ?? ""):\(model.cwd)" : nil) {
                guard model.location == .resume, model.folder != nil else { return }
                try? await Task.sleep(for: .milliseconds(250))
                guard !Task.isCancelled else { return }
                await model.loadSessions()
            }
            .sheet(isPresented: $pickingPrompt) {
                NavigationStack {
                    PromptShortcutsView { shortcut in
                        guard model.phase == .editing, !model.dictation.isActive else { return }
                        model.prompt = shortcut.appending(to: model.prompt)
                        editingPrompt = true
                    }
                }
            }
            .modifier(AttachmentPickers(pickingPhoto: $pickingPhoto, pickingFile: $pickingFile, slots: model.attachmentSlots) {
                model.attach($0)
            } failed: {
                model.attachFailed($0)
            })
            .onChange(of: scenePhase) { _, phase in model.dictation.scenePhaseChanged(to: phase) }
            .onChange(of: model.prompt.isEmpty) { _, empty in
                if empty && !model.dictation.isActive { dictationExpanded = false }
            }
            .onChange(of: model.dictation.isActive) { _, active in
                if !active && model.prompt.isEmpty { dictationExpanded = false }
            }
        }
        .onDisappear {
            dictationExpanded = false
            cancel()
        }
    }

    @ViewBuilder
    private func folderPicker(_ path: Binding<String>, completions: [String]) -> some View {
        TextField(model.base.map { "Name in \($0) or /absolute/path" } ?? "/path/to/project", text: path)
            .textInputAutocapitalization(.never)
            .autocorrectionDisabled()
            .font(.body.monospaced())
        if let folders = model.folders {
            ForEach(completions, id: \.self) { name in
                Button {
                    path.wrappedValue = folders.folderPath(name)
                } label: {
                    Label(name, systemImage: "folder").font(.footnote.monospaced())
                }
                .foregroundStyle(.primary)
            }
            NavigationLink {
                FolderBrowser(folders: folders, selected: path)
            } label: {
                Text("Browse \(folders.path)").lineLimit(1).truncationMode(.head)
            }
        }
        ForEach(model.options?.recentCwds ?? [], id: \.self) { cwd in
            Button {
                path.wrappedValue = cwd
            } label: {
                HStack {
                    Text(cwd).font(.footnote.monospaced()).lineLimit(1).truncationMode(.head)
                    Spacer()
                    if path.wrappedValue == cwd {
                        Image(systemName: "checkmark").foregroundStyle(.primary)
                    }
                }
            }
            .foregroundStyle(.primary)
        }
    }
}

/// A rotation moves the keyboard without scrolling the focused prompt back above it.
private struct KeepsPromptVisible: ViewModifier {
    let editing: Bool

    func body(content: Content) -> some View {
        ScrollViewReader { proxy in
            content.onReceive(NotificationCenter.default.publisher(for: UIResponder.keyboardDidChangeFrameNotification)) { _ in
                if editing { withAnimation { proxy.scrollTo(KeepsPromptVisible.id, anchor: .bottom) } }
            }
        }
    }

    static let id = "prompt"
}

private struct FolderBrowser: View {
    let folders: TaskFolders
    @Binding var selected: String
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        List {
            Section {
                ForEach(folders.folders, id: \.self) { name in
                    Button {
                        selected = folders.folderPath(name)
                        dismiss()
                    } label: {
                        Label(name, systemImage: "folder").font(.body.monospaced())
                    }
                    .foregroundStyle(.primary)
                }
            } footer: {
                if folders.folders.isEmpty {
                    Text("No folders.")
                } else if folders.truncated {
                    Text("Only the first \(folders.folders.count) folders are listed. Type a name to reach the others.")
                }
            }
        }
        .navigationTitle(URL(filePath: folders.path).lastPathComponent)
        .navigationBarTitleDisplayMode(.inline)
    }
}
