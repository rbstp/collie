import CollieCore
import SwiftUI

struct NewTaskSheet: View {
    let machines: [Machine]
    let planUsage: [String: PlanUsage]
    let onStarted: (AgentRoute, Bool) -> Void
    @State private var model: NewTaskModel
    @State private var startTask: Task<Void, Never>?
    @State private var pickingPhoto = false
    @State private var pickingFile = false
    @State private var pickingPrompt = false
    @State private var dictationExpanded = false
    @State private var dictationShown = false
    @State private var selection: TextSelection?
    @State private var now = Date.now
    @FocusState private var editingPrompt: Bool
    @State private var naming = false
    @FocusState private var editingLabel: Bool
    @Environment(\.appAccent) private var appAccent
    @Environment(\.dismiss) private var dismiss
    @Environment(\.scenePhase) private var scenePhase
    @Environment(\.verticalSizeClass) private var verticalSizeClass

    private var expandedPrompt: Bool {
        editingPrompt || model.dictation.isActive || dictationShown || !model.attachments.isEmpty || (dictationExpanded && !model.prompt.isEmpty)
    }

    init(
        core: any AgentCore, machines: [Machine], preferredMachineId: String?, planUsage: [String: PlanUsage],
        onStarted: @escaping (AgentRoute, Bool) -> Void
    ) {
        self.init(
            model: NewTaskModel(core: core, machines: machines, preferredMachineId: preferredMachineId, draftsFile: AgentDrafts.file, scheduledFile: ScheduledDrafts.file),
            machines: machines, planUsage: planUsage, onStarted: onStarted
        )
    }

    init(model: NewTaskModel, machines: [Machine], planUsage: [String: PlanUsage] = [:], onStarted: @escaping (AgentRoute, Bool) -> Void) {
        self.machines = machines
        self.planUsage = planUsage
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
                if let error = model.error {
                    Label(error, systemImage: "xmark.octagon").foregroundStyle(.red)
                }

                Section {
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
                        PromptInputLayout(expanded: expandedPrompt && verticalSizeClass != .compact || model.dictation.isActive || dictationShown) {
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
                                let placeholder = model.location == .resume ? "Prompt" : "What should the agent do?"
                                TextField(placeholder, text: $model.prompt, selection: $selection, prompt: Text(placeholder).foregroundStyle(.metadata), axis: .vertical)
                                    .lineLimit(verticalSizeClass == .compact ? (expandedPrompt ? 1...2 : 1...1) : (expandedPrompt ? 1...5 : 1...3))
                                    .focused($editingPrompt)
                                    .disabled(model.dictation.isActive || (model.editing != nil && model.phase == .starting))
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
                            DictationControl(dictation: model.dictation, disabled: model.phase != .editing, dim: .never, shown: $dictationShown) {
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
                                    .appGlassButton(dim: .never)
                                    .tint(.primary)
                                    .buttonBorderShape(.circle)
                                    .controlSize(.small)
                                    .accessibilityLabel("Hide keyboard")
                                }
                            }
                        }
                    }
                    .id(KeepsFieldVisible.prompt)
                }
                .listRowInsets(.vertical, 8)

                if model.editing == nil {
                    Section {
                        if machines.count > 1 {
                            Picker("Machine", selection: $model.machineId) {
                                ForEach(machines, id: \.id) { machine in
                                    Text(machine.label).tag(Optional(machine.id))
                                }
                            }
                            .tint(.primary)
                        }
                        Picker("Location", selection: $model.location) {
                            ForEach(model.locations, id: \.self) { location in
                                Text(location.rawValue).tag(location)
                            }
                        }
                        .tint(.primary)
                        if model.location == .folder {
                            Toggle("New folder", isOn: $model.newFolder)
                            if model.newFolder {
                                if model.base == nil, let roots = model.options?.roots, roots.count > 1 {
                                    Picker("In", selection: Binding(get: { model.newFolderParent }, set: { model.newFolderRoot = $0 })) {
                                        ForEach(roots, id: \.self) { Text($0).tag(Optional($0)) }
                                    }
                                    .tint(.primary)
                                }
                                TextField("Folder name", text: $model.folderName, prompt: Text("Folder name").foregroundStyle(.metadata))
                                    .textInputAutocapitalization(.never)
                                    .autocorrectionDisabled()
                                    .font(.subheadline.monospaced())
                            } else {
                                folderPicker($model.cwd, completions: model.completions)
                            }
                        } else if model.location == .clone {
                            if model.base == nil, let roots = model.options?.roots, roots.count > 1 {
                                Picker("In", selection: Binding(get: { model.newFolderParent }, set: { model.newFolderRoot = $0 })) {
                                    ForEach(roots, id: \.self) { Text($0).tag(Optional($0)) }
                                }
                                .tint(.primary)
                            }
                            TextField("owner/repo or URL", text: $model.repository, prompt: Text("owner/repo or URL").foregroundStyle(.metadata))
                                .textInputAutocapitalization(.never)
                                .autocorrectionDisabled()
                                .keyboardType(.URL)
                                .font(.subheadline.monospaced())
                        } else if model.location == .resume {
                            folderPicker($model.cwd, completions: model.completions)
                        } else {
                            if model.location == .worktree {
                                Picker("Action", selection: $model.worktreeAction) {
                                    ForEach(NewTaskModel.WorktreeAction.allCases, id: \.self) { action in
                                        Text(action.rawValue).tag(action)
                                    }
                                }
                                .tint(.primary)
                            }
                            folderPicker($model.source, completions: model.sourceCompletions)
                            if model.worktrees == nil, model.worktreesError == nil, model.sourceFolder != nil {
                                ProgressView("Checking repository…")
                            }
                            if let listed = model.worktrees {
                                if model.location != .issue, listed.source != model.sourceFolder {
                                    Text("Repository: \(listed.source)").font(.footnote.monospaced())
                                }
                                if model.location == .issue {
                                    if let issues = model.issues {
                                        if issues.isEmpty {
                                            Text("No open issue in this repository.").font(.footnote).foregroundStyle(.metadata)
                                        }
                                        if !issues.isEmpty {
                                            VStack(spacing: 0) {
                                                ForEach(issues, id: \.number) { issue in
                                                    Button {
                                                        model.pick(issue)
                                                    } label: {
                                                        HStack(spacing: 6) {
                                                            Text("#\(issue.number)").monospacedDigit().foregroundStyle(.metadata)
                                                            Text(issue.title).lineLimit(1)
                                                            Spacer(minLength: 0)
                                                            if model.issue == issue.number { Image(systemName: "checkmark").foregroundStyle(.tint) }
                                                        }
                                                        .font(.footnote)
                                                        .padding(.vertical, 6)
                                                        .contentShape(Rectangle())
                                                    }
                                                    .buttonStyle(.plain)
                                                    .disabled(model.phase != .editing || model.dictation.isActive)
                                                }
                                            }
                                        }
                                    } else if model.issuesError == nil {
                                        ProgressView("Finding issues…")
                                    }
                                    if let branch = model.issueBranch {
                                        Text("Creates \(branch) under .worktree/").font(.footnote).foregroundStyle(.metadata)
                                    }
                                    if let error = model.issuesError { Text(error).foregroundStyle(.red) }
                                } else if model.worktreeAction == .create {
                                    TextField("Branch name", text: $model.branch, prompt: Text("Branch name").foregroundStyle(.metadata))
                                        .textInputAutocapitalization(.never)
                                        .autocorrectionDisabled()
                                    if model.createPath != nil {
                                        Text("Creates \(model.createBranch) under .worktree/")
                                            .font(.footnote).foregroundStyle(.metadata)
                                    }
                                } else if model.worktreeAction == .pullRequest {
                                    TextField("Pull request number", text: $model.pullRequestNumber, prompt: Text("Pull request number").foregroundStyle(.metadata))
                                        .keyboardType(.numberPad)
                                    if let number = model.pullRequest {
                                        Text("Checks out #\(number) under .worktree/")
                                            .font(.footnote).foregroundStyle(.metadata)
                                    }
                                } else {
                                    ForEach(listed.worktrees, id: \.path) { checkout in
                                        Button {
                                            model.checkoutPath = checkout.path
                                        } label: {
                                            HStack {
                                                VStack(alignment: .leading) {
                                                    Text(checkout.branch ?? "Detached")
                                                    Text(checkout.path).font(.caption.monospaced()).foregroundStyle(.metadata)
                                                }
                                                Spacer()
                                                if checkout.open { Text("Open").font(.footnote) }
                                                if model.checkoutPath == checkout.path { Image(systemName: "checkmark").foregroundStyle(.tint) }
                                            }
                                        }
                                        .foregroundStyle(.primary)
                                    }
                                }
                            }
                            if let error = model.worktreesError { Text(error).foregroundStyle(.red) }
                        }
                    } footer: {
                        let parent = model.newFolderParent
                        let name = model.folderName.trimmingCharacters(in: .whitespacesAndNewlines)
                        let note: String? =
                            if model.location == .folder, model.newFolder, let parent {
                                "Creates \(parent)/\(name.isEmpty ? "name" : name) (empty) and starts the agent there."
                            } else if model.location == .clone, let parent {
                                "Clones into \(parent)/\(model.cloneFolder ?? "repo") and starts the agent there."
                            } else {
                                nil
                            }
                        let errors = [model.optionsError, model.runtimeProblem, model.location != .clone ? model.foldersError : nil].compactMap(\.self)
                        // An empty footer still takes room in a Form, which widened the gap to the next section.
                        if note != nil || !errors.isEmpty {
                            VStack(alignment: .leading, spacing: 4) {
                                if let note { Text(note) }
                                ForEach(errors, id: \.self) { Text($0).foregroundStyle(.red) }
                            }
                        }
                    }
                    .listRowInsets(.vertical, 8)

                    if model.location == .resume {
                        Section("Session") {
                            if model.sessions == nil, model.sessionsError == nil, model.folder != nil {
                                ProgressView("Finding sessions…")
                            }
                            if let sessions = model.resumable {
                                if sessions.isEmpty {
                                    Text("No session to resume in this folder.").foregroundStyle(.metadata)
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
                                                .foregroundStyle(.metadata)
                                            }
                                            Spacer()
                                            if model.session?.sessionId == session.sessionId { Image(systemName: "checkmark").foregroundStyle(.tint) }
                                        }
                                    }
                                    .foregroundStyle(.primary)
                                }
                            }
                            if let error = model.sessionsError { Text(error).foregroundStyle(.red) }
                        }
                        .listRowInsets(.vertical, 8)
                    }

                    Section {
                        if model.location != .resume {
                            if let agents = model.offeredAgents, !agents.isEmpty {
                                Picker(selection: $model.agent) {
                                    ForEach(agents, id: \.self) { AgentKindLabel(kind: $0).tag($0) }
                                } label: {
                                    Text("Agent")
                                } currentValueLabel: {
                                    // The menu's value drops the label's spacing, so the logo touched the name; the title carries the space.
                                    if let display = AgentKindLabel.display(model.agent) {
                                        Label(" \(display.title)", image: display.image)
                                            .accessibilityLabel(display.title)
                                    } else {
                                        Label(" \(model.agent.capitalized)", systemImage: "terminal")
                                            .accessibilityLabel(model.agent.capitalized)
                                    }
                                }
                                .tint(.primary)
                                // A row rather than the footer: light orange is 4.1:1 on the grouped background, 4.5:1 on the row.
                                // A TimelineView here would leave an empty row without a warning.
                                if let warning = model.machineId.flatMap({ planUsage[$0] })?.warning(agent: model.agent, now: now) {
                                    Text(warning.text)
                                        .font(.footnote)
                                        .foregroundStyle(ContextRing.tone(100 - warning.used))
                                        .accessibilityLabel(warning.spoken)
                                }
                            } else if model.options == nil, model.optionsError == nil {
                                ProgressView()
                            }
                        }
                        HStack(spacing: 12) {
                            if model.location != .resume, model.offeredAgents?.isEmpty == false, model.agent == "claude" {
                                optionToggle("Ultracode", systemImage: "sparkles", isOn: $model.ultracode)
                            }
                            if model.offersCodeMode {
                                optionToggle("Code Mode", systemImage: "chevron.left.forwardslash.chevron.right", isOn: $model.codeMode)
                            }
                            if model.offersSchedule || model.startLater {
                                optionToggle("Start later", systemImage: "clock", isOn: $model.startLater)
                            }
                            optionToggle("Name workspace", systemImage: "tag", isOn: Binding(get: { naming }, set: { on in
                                naming = on
                                editingLabel = on
                                if !on { model.label = "" }
                            }))
                        }
                        if model.scheduleUnsupported {
                            Text(NewTaskModel.scheduleTooOld).font(.footnote).foregroundStyle(.red)
                        } else if model.startLater {
                            DatePicker("Start at", selection: $model.startAt, in: Date.now...Date.now.addingTimeInterval(NewTaskModel.scheduleAhead))
                        }
                        if naming {
                            TextField("Workspace label", text: $model.label, prompt: Text("Workspace label").foregroundStyle(.metadata))
                                .focused($editingLabel)
                                .id(KeepsFieldVisible.label)
                        }
                    }
                    .listRowInsets(.vertical, 8)

                } else {
                    Section {
                        DatePicker("Start at", selection: $model.startAt, in: Date.now...Date.now.addingTimeInterval(NewTaskModel.scheduleAhead))
                    } footer: {
                        Text("The folder, agent and place stay as scheduled. To change them, cancel the task and schedule it again.")
                    }
                    .listRowInsets(.vertical, 8)
                }

                if model.editing == nil, !model.scheduled.isEmpty {
                    Section {
                        ForEach(model.scheduled, id: \.scheduleId) { task in
                            Button {
                                model.edit(task)
                            } label: {
                                HStack {
                                    VStack(alignment: .leading, spacing: 2) {
                                        Text(URL(filePath: task.cwd).lastPathComponent)
                                        Text(Date(timeIntervalSince1970: Double(task.startAtMs) / 1000), format: .dateTime.weekday(.abbreviated).hour().minute())
                                            .font(.footnote)
                                            .foregroundStyle(.metadata)
                                    }
                                    Spacer()
                                    AgentKindLabel(kind: task.agent)
                                }
                            }
                            .foregroundStyle(.primary)
                            .swipeActions {
                                Button("Cancel", role: .destructive) {
                                    Task { await model.unschedule(task) }
                                }
                            }
                        }
                    } header: {
                        Text("Scheduled on \(machines.first { $0.id == model.machineId }?.label ?? "this computer")")
                    }
                    .listRowInsets(.vertical, 8)
                }
            }
            .listSectionSpacing(.compact)
            .environment(\.defaultMinListRowHeight, 44)
            .contentMargins(.top, 6, for: .scrollContent)
            .safeAreaPadding(.bottom, editingPrompt ? 48 : 0)
            .modifier(KeepsFieldVisible(row: editingPrompt ? KeepsFieldVisible.prompt : editingLabel ? KeepsFieldVisible.label : nil))
            .safeAreaInset(edge: .top) {
                if let repository = model.cloning {
                    HStack(spacing: 10) {
                        ProgressView().controlSize(.small)
                        Text("Cloning \(repository)…")
                    }
                    .font(.footnote)
                    .padding(12)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .appGlass(in: .rect(cornerRadius: 12))
                    .padding(.horizontal)
                    .padding(.vertical, 6)
                    .allowsHitTesting(false)
                }
            }
            .navigationTitle(model.editing == nil ? "New task" : "Edit scheduled task")
            .navigationSubtitle(model.phase == .starting ? "Starting the task…" : model.phase == .waitingForAgent ? "Waiting for the agent…" : "")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button(model.phase == .startedNotVisible ? "Done" : "Cancel") {
                        if model.editing != nil {
                            model.stopEditing()
                        } else {
                            cancel()
                            dismiss()
                        }
                    }
                    .disabled(model.editing != nil && model.phase == .starting)
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button {
                        startTask = Task {
                            if model.editing != nil {
                                _ = await model.saveEdit()
                            } else if model.startLater {
                                if await model.schedule() { dismiss() }
                            } else if let route = await model.start() {
                                dismiss()
                                onStarted(route, model.startupBlocked)
                            }
                        }
                    } label: {
                        if model.phase == .starting || model.phase == .waitingForAgent {
                            ProgressView()
                        } else {
                            Text(model.editing != nil ? "Save" : model.startLater ? "Schedule" : "Start").onAccentFill()
                        }
                    }
                    .buttonStyle(.glassProminent)
                    .accessibilityLabel(model.editing != nil ? "Save" : model.startLater ? "Schedule" : "Start")
                    .disabled(model.editing != nil ? !model.canSave : !model.canStart)
                }
            }
            .task(id: model.machineId) {
                await model.loadOptions()
                await model.loadScheduled()
            }
            .task {
                while !Task.isCancelled {
                    try? await Task.sleep(for: .seconds(60))
                    now = .now
                }
            }
            .task(id: model.location == .worktree || model.location == .issue ? "\(model.machineId ?? ""):\(model.location.rawValue):\(model.worktreeAction.rawValue):\(model.base ?? ""):\(model.source)" : nil) {
                guard model.location == .worktree || model.location == .issue, model.sourceFolder != nil else {
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

    private var missingFolder: Bool {
        model.location == .folder && !model.newFolder && model.folder == nil
            && (!model.prompt.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || !model.attachments.isEmpty)
    }

    private func optionToggle(_ title: String, systemImage: String, isOn: Binding<Bool>) -> some View {
        Button {
            isOn.wrappedValue.toggle()
        } label: {
            Image(systemName: systemImage)
                .font(.system(size: 17, weight: .medium))
                .frame(width: 28, height: 28)
        }
        .appGlassButton(dim: .never)
        .tint(isOn.wrappedValue ? appAccent : .primary)
        .buttonBorderShape(.circle)
        .controlSize(.small)
        .accessibilityLabel(title)
        .accessibilityAddTraits(isOn.wrappedValue ? .isSelected : [])
    }

    @ViewBuilder
    private func folderPicker(_ path: Binding<String>, completions: [String]) -> some View {
        // The hint takes the placeholder's place, so the form keeps its geometry when a folder is chosen.
        TextField(
            "Folder", text: path,
            prompt: missingFolder
                ? Text("Choose a folder to start").font(.subheadline).foregroundStyle(ContextRing.tone(20))
                : Text(model.base.map { "Name in \($0) or /absolute/path" } ?? "/path/to/project").foregroundStyle(.metadata)
        )
        .textInputAutocapitalization(.never)
        .autocorrectionDisabled()
        .font(.subheadline.monospaced())
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
                    VStack(alignment: .leading, spacing: 2) {
                        Text(URL(filePath: cwd).lastPathComponent)
                        // A long path drops whole leading folders rather than cutting one mid-word.
                        ViewThatFits(in: .horizontal) {
                            let parts = cwd.split(separator: "/")
                            ForEach(0..<parts.count, id: \.self) { dropped in
                                Text(verbatim: dropped == 0 ? cwd : "…/" + parts.dropFirst(dropped).joined(separator: "/"))
                            }
                        }
                        .font(.caption.monospaced()).foregroundStyle(.metadata).lineLimit(1).truncationMode(.head)
                    }
                    Spacer()
                    if path.wrappedValue == cwd {
                        Image(systemName: "checkmark").foregroundStyle(.tint)
                    }
                }
            }
            .foregroundStyle(.primary)
        }
    }
}

/// A rotation moves the keyboard without scrolling the focused prompt back above it, and UIKit scrolls a revealed field
/// only until its text clears the keyboard, leaving the row's bottom under it.
private struct KeepsFieldVisible: ViewModifier {
    let row: String?
    @State private var current: String?
    @State private var userScrolling = false

    func body(content: Content) -> some View {
        ScrollViewReader { proxy in
            content.onReceive(NotificationCenter.default.publisher(for: UIResponder.keyboardDidChangeFrameNotification)) { _ in
                if let row { proxy.scrollTo(row, anchor: .bottom) }
            }
            .onChange(of: row, initial: true) { current = row }
            .onScrollPhaseChange { _, phase in userScrolling = phase.isScrolling }
            // After a rotation the notification can arrive before the form takes the keyboard's inset. That scroll then left
            // the prompt row where UIKit keeps only the field visible, under the predictive bar. The prompt row can also take
            // its landscape height after the last scroll, and the list then moved it by the difference, under the navigation
            // bar. A scroll made while the sheet still animates into its rotated frame is measured against a form about 80
            // points tall and can land last, leaving the row under the navigation bar with only the offset changed, so an
            // offset change the user did not make scrolls again. An animated scroll restarted on every offset change barely
            // moves. Scrolling from inside this callback was measured against the old layout, hence the next turn of the main
            // queue, where a drag or a focus change may have come first.
            .onScrollGeometryChange(for: [CGFloat].self) {
                [$0.containerSize.height, $0.contentInsets.bottom, $0.contentSize.height, $0.contentOffset.y]
            } action: { old, new in
                let onlyOffset = old.dropLast() == new.dropLast()
                guard let row, !(onlyOffset && userScrolling) else { return }
                DispatchQueue.main.async {
                    guard current == row, !(onlyOffset && userScrolling) else { return }
                    proxy.scrollTo(row, anchor: .bottom)
                }
            }
        }
    }

    static let prompt = "prompt"
    static let label = "label"
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
