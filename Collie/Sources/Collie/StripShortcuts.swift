import CollieCore
import SwiftUI

/// A button of the key strip: one key (with its modifier, so never more than two keys), or text sent at once.
struct StripShortcut: Codable, Identifiable, Equatable {
    var id = UUID()
    var name = ""
    /// collied's key name; nil for a text shortcut.
    var key: String?
    var text = ""
    /// nil for every agent and shell, else one kind: "claude", "codex", "copilot" or "terminal".
    var kind: String?

    var agentKey: AgentKey? { key.flatMap(AgentKey.init(name:)) }

    var label: String {
        if !name.isEmpty { return name }
        return agentKey?.symbol ?? text
    }

    func applies(to kind: String?) -> Bool { self.kind == nil || self.kind == kind }

    static let kinds: [(String, String?)] = [
        ("All", nil), ("Claude Code", "claude"), ("Codex", "codex"), ("Copilot CLI", "copilot"), ("Terminal", "terminal"),
    ]
}

struct StripShortcuts: StateFile {
    static let file: URL? = try? StateDirectory.prepare().appending(path: "strip-shortcuts.json")
    var items: [StripShortcut] = []
}

struct StripShortcutsView: View {
    @State private var shortcuts = StripShortcuts.load(from: StripShortcuts.file)
    @State private var editing: StripShortcut?
    var done: (() -> Void)?

    var body: some View {
        List {
            if shortcuts.items.isEmpty {
                ContentUnavailableView(
                    "No key shortcuts", systemImage: "arrowkeys",
                    description: Text("Add a key combination or a command to send it from the key strip in one tap.")
                )
            }
            ForEach(shortcuts.items) { shortcut in
                Button {
                    editing = shortcut
                } label: {
                    VStack(alignment: .leading, spacing: 4) {
                        Text(shortcut.label).font(.headline).foregroundStyle(.primary)
                        Text(summary(shortcut)).font(.subheadline).foregroundStyle(.secondary).lineLimit(2)
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
            }
            .onDelete { offsets in
                shortcuts.items.remove(atOffsets: offsets)
                shortcuts.save(to: StripShortcuts.file)
            }
            .onMove { from, to in
                shortcuts.items.move(fromOffsets: from, toOffset: to)
                shortcuts.save(to: StripShortcuts.file)
            }
        }
        .navigationTitle("Key shortcuts")
        .toolbar {
            ToolbarItem(placement: .primaryAction) {
                Button("Add shortcut", systemImage: "plus") { editing = StripShortcut(key: AgentKey.shiftTab.name) }
            }
            if !shortcuts.items.isEmpty {
                ToolbarItem(placement: .primaryAction) { EditButton() }
            }
            if let done {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Done", action: done)
                }
            }
        }
        .sheet(item: $editing) { shortcut in
            NavigationStack {
                StripShortcutEditor(shortcut: shortcut) { saved in
                    if let index = shortcuts.items.firstIndex(where: { $0.id == saved.id }) {
                        shortcuts.items[index] = saved
                    } else {
                        shortcuts.items.append(saved)
                    }
                    shortcuts.save(to: StripShortcuts.file)
                }
            }
        }
        .onAppear { shortcuts = StripShortcuts.load(from: StripShortcuts.file) }
    }

    private func summary(_ shortcut: StripShortcut) -> String {
        let what = shortcut.agentKey?.accessibilityName ?? shortcut.text
        let kind = StripShortcut.kinds.first { $0.1 == shortcut.kind }?.0 ?? "All"
        return "\(what) · \(kind)"
    }
}

private struct StripShortcutEditor: View {
    @State var shortcut: StripShortcut
    let save: (StripShortcut) -> Void
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        Form {
            TextField("Name (optional)", text: $shortcut.name)
            Picker("Sends", selection: Binding(
                get: { shortcut.key != nil },
                set: { shortcut.key = $0 ? AgentKey.shiftTab.name : nil }
            )) {
                Text("Keys").tag(true)
                Text("Text").tag(false)
            }
            .pickerStyle(.segmented)
            if shortcut.key != nil {
                Picker("Keys", selection: Binding(get: { shortcut.key ?? "" }, set: { shortcut.key = $0 })) {
                    ForEach(AgentKey.all, id: \.self) { key in
                        Text("\(key.symbol)  \(key.accessibilityName)").tag(key.name)
                    }
                }
                .tint(.primary)
            } else {
                Section {
                    TextField("/effort high", text: $shortcut.text, axis: .vertical)
                        .lineLimit(1...6)
                        .autocorrectionDisabled()
                        .textInputAutocapitalization(.never)
                } footer: {
                    Text("Sent at once, only while the agent’s input box is empty. In a terminal, it runs as a command.")
                }
            }
            Picker("Agents", selection: $shortcut.kind) {
                ForEach(StripShortcut.kinds, id: \.0) { Text($0.0).tag($0.1) }
            }
            .tint(.primary)
        }
        .navigationTitle("Key shortcut")
        .toolbar {
            ToolbarItem(placement: .cancellationAction) {
                Button("Cancel") { dismiss() }
            }
            ToolbarItem(placement: .confirmationAction) {
                Button("Save") {
                    shortcut.name = shortcut.name.trimmingCharacters(in: .whitespacesAndNewlines)
                    shortcut.text = shortcut.key == nil ? shortcut.text.trimmingCharacters(in: .whitespacesAndNewlines) : ""
                    save(shortcut)
                    dismiss()
                }
                .disabled(shortcut.key == nil && shortcut.text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
            }
        }
    }
}
