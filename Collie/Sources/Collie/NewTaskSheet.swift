import CollieCore
import SwiftUI

struct NewTaskSheet: View {
    let machines: [Machine]
    let onStarted: (AgentRoute) -> Void
    @State private var model: NewTaskModel
    @State private var startTask: Task<Void, Never>?
    @Environment(\.dismiss) private var dismiss

    init(
        core: any AgentCore, machines: [Machine], preferredMachineId: String?,
        onStarted: @escaping (AgentRoute) -> Void
    ) {
        self.machines = machines
        self.onStarted = onStarted
        _model = State(initialValue: NewTaskModel(core: core, machines: machines, preferredMachineId: preferredMachineId))
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
                }

                Section {
                    Toggle("New folder", isOn: $model.newFolder)
                    if model.newFolder {
                        if model.base == nil, let roots = model.options?.roots, roots.count > 1 {
                            Picker("In", selection: Binding(get: { model.newFolderParent }, set: { model.newFolderRoot = $0 })) {
                                ForEach(roots, id: \.self) { Text($0).tag(Optional($0)) }
                            }
                        }
                        TextField("Folder name", text: $model.folderName)
                            .textInputAutocapitalization(.never)
                            .autocorrectionDisabled()
                            .font(.body.monospaced())
                    } else {
                        TextField(model.base.map { "Name in \($0) or /absolute/path" } ?? "/path/to/project", text: $model.cwd)
                            .textInputAutocapitalization(.never)
                            .autocorrectionDisabled()
                            .font(.body.monospaced())
                        ForEach(model.completions, id: \.self) { name in
                            Button {
                                model.cwd = name
                            } label: {
                                Label(name, systemImage: "folder").font(.footnote.monospaced())
                            }
                            .foregroundStyle(.primary)
                        }
                        if let folders = model.folders {
                            NavigationLink {
                                FolderBrowser(folders: folders, selected: $model.cwd)
                            } label: {
                                Text("Browse \(folders.path)").lineLimit(1).truncationMode(.head)
                            }
                        }
                        ForEach(model.options?.recentCwds ?? [], id: \.self) { cwd in
                            Button {
                                model.cwd = cwd
                            } label: {
                                HStack {
                                    Text(cwd).font(.footnote.monospaced()).lineLimit(1).truncationMode(.head)
                                    Spacer()
                                    if model.cwd == cwd {
                                        Image(systemName: "checkmark").foregroundStyle(.tint)
                                    }
                                }
                            }
                            .foregroundStyle(.primary)
                        }
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

                Section {
                    if let agents = model.options?.agents {
                        Picker("Agent", selection: $model.agent) {
                            ForEach(agents, id: \.self) { AgentKindLabel(kind: $0).tag($0) }
                        }
                    } else if model.optionsError == nil {
                        ProgressView()
                    }
                } header: {
                    Text("Agent")
                } footer: {
                    if !model.agent.isEmpty, model.agent != "claude" {
                        Text("Approve and Deny are for Claude Code: this agent's prompts are answered in the terminal on the machine.")
                    }
                }

                Section("Prompt") {
                    TextField("What should the agent do?", text: $model.prompt, axis: .vertical)
                        .lineLimit(3...10)
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
            .onDisappear(perform: cancel)
        }
    }
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
                        selected = name
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
