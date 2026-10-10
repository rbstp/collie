import SwiftUI

struct PromptShortcut: Codable, Identifiable {
    var id = UUID()
    var name = ""
    var text = ""

    func appending(to draft: String) -> String {
        draft.isEmpty ? text : draft + "\n\n" + text
    }
}

struct PromptShortcuts: StateFile {
    static let file: URL? = try? StateDirectory.prepare().appending(path: "prompt-shortcuts.json")
    var items: [PromptShortcut] = []
}

struct PromptShortcutsView: View {
    var select: ((PromptShortcut) -> Void)?
    @State private var shortcuts = PromptShortcuts.load(from: PromptShortcuts.file)
    @State private var editing: PromptShortcut?
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        List {
            if shortcuts.items.isEmpty {
                ContentUnavailableView("No saved prompts", systemImage: "text.badge.plus", description: Text("Add a prompt to reuse it later."))
            }
            ForEach(shortcuts.items) { shortcut in
                Button {
                    if let select {
                        select(shortcut)
                        dismiss()
                    } else {
                        editing = shortcut
                    }
                } label: {
                    VStack(alignment: .leading, spacing: 4) {
                        Text(shortcut.name).foregroundStyle(.primary)
                        Text(shortcut.text).font(.caption).foregroundStyle(.secondary).lineLimit(2)
                    }
                }
                .swipeActions {
                    Button("Delete", role: .destructive) {
                        shortcuts.items.removeAll { $0.id == shortcut.id }
                        shortcuts.save(to: PromptShortcuts.file)
                    }
                    Button("Edit") { editing = shortcut }.tint(.blue)
                }
            }
        }
        .navigationTitle("Saved prompts")
        .toolbar {
            ToolbarItem(placement: .primaryAction) {
                Button("Add prompt", systemImage: "plus") { editing = PromptShortcut() }
            }
            if select != nil {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { dismiss() }
                }
            }
        }
        .sheet(item: $editing) { shortcut in
            NavigationStack {
                PromptShortcutEditor(shortcut: shortcut) { saved in
                    if let index = shortcuts.items.firstIndex(where: { $0.id == saved.id }) {
                        shortcuts.items[index] = saved
                    } else {
                        shortcuts.items.append(saved)
                    }
                    shortcuts.save(to: PromptShortcuts.file)
                }
            }
        }
        .onAppear { shortcuts = PromptShortcuts.load(from: PromptShortcuts.file) }
    }
}

private struct PromptShortcutEditor: View {
    @State var shortcut: PromptShortcut
    let save: (PromptShortcut) -> Void
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        Form {
            TextField("Name", text: $shortcut.name)
            Section("Prompt") {
                TextField("Prompt text", text: $shortcut.text, axis: .vertical)
                    .lineLimit(5...12)
            }
        }
        .navigationTitle("Saved prompt")
        .toolbar {
            ToolbarItem(placement: .cancellationAction) {
                Button("Cancel") { dismiss() }
            }
            ToolbarItem(placement: .confirmationAction) {
                Button("Save") {
                    shortcut.name = shortcut.name.trimmingCharacters(in: .whitespacesAndNewlines)
                    save(shortcut)
                    dismiss()
                }
                .disabled(shortcut.name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
                    || shortcut.text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
            }
        }
    }
}
