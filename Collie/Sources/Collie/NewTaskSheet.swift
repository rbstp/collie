import CollieCore
import SwiftUI

struct NewTaskSheet: View {
    let machines: [Machine]
    let onStarted: (AgentRoute) -> Void
    @State private var model: NewTaskModel
    @State private var startTask: Task<Void, Never>?
    @Environment(\.dismiss) private var dismiss

    init(core: any AgentCore, machines: [Machine], onStarted: @escaping (AgentRoute) -> Void) {
        self.machines = machines
        self.onStarted = onStarted
        _model = State(initialValue: NewTaskModel(core: core, machines: machines))
    }

    var body: some View {
        NavigationStack {
            Form {
                if machines.count > 1 {
                    Picker("Mac", selection: $model.machineId) {
                        ForEach(machines, id: \.id) { machine in
                            Text(machine.label).tag(Optional(machine.id))
                        }
                    }
                }

                Section {
                    TextField("/Users/you/project", text: $model.cwd)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        .font(.body.monospaced())
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
                } header: {
                    Text("Folder")
                } footer: {
                    if let error = model.optionsError {
                        Text(error).foregroundStyle(.red)
                    }
                }

                Section("Agent") {
                    if let agents = model.options?.agents {
                        Picker("Agent", selection: $model.agent) {
                            ForEach(agents, id: \.self) { Text($0).tag($0) }
                        }
                    } else if model.optionsError == nil {
                        ProgressView()
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
                            if let route = await model.start(), !Task.isCancelled {
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
                    Button(model.phase == .startedNotVisible ? "Done" : "Cancel") { dismiss() }
                }
            }
            .task(id: model.machineId) { await model.loadOptions() }
            .onDisappear { startTask?.cancel() }
        }
    }
}
