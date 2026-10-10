import CollieCore
import SwiftUI

struct GitHunk: Identifiable, Equatable {
    let id: Int
    let text: String
    let complete: Bool

    func feedback(root: String, file: GitFile) -> String {
        var fence = "```"
        while text.contains(fence) { fence += "`" }
        let renamed = file.oldPath.map { "\nRenamed from: \($0)" } ?? ""
        return "Review feedback\nCheckout: \(root)\nFile: \(file.path)\(renamed)\nChanges: \(file.section)\n\n\(fence)diff\n\(text)\(text.hasSuffix("\n") ? "" : "\n")\(fence)\n\nFeedback: "
    }
}

struct GitPatch {
    let header: String
    let hunks: [GitHunk]

    init(_ diff: GitDiff) {
        let lines = diff.patch.components(separatedBy: "\n")
        var header = ""
        var chunks: [String] = []
        for (index, line) in lines.enumerated() {
            let text = String(line) + (index < lines.count - 1 ? "\n" : "")
            if line.hasPrefix("@@ ") { chunks.append(text) }
            else if chunks.isEmpty { header += text }
            else { chunks[chunks.count - 1] += text }
        }
        self.header = header
        hunks = chunks.enumerated().map { index, text in
            GitHunk(id: index, text: text, complete: !diff.truncated || index < chunks.count - 1)
        }
    }
}

struct GitChangesView: View {
    let agent: AgentModel
    let drafted: () -> Void
    @Environment(\.dismiss) private var dismiss
    @State private var changes: GitChanges?
    @State private var error: String?
    @State private var loading = false

    var body: some View {
        NavigationStack {
            List {
                if let error { Text(error).foregroundStyle(.red) }
                if let changes {
                    if let root = changes.root {
                        Section {
                            Label(changes.branch ?? "Detached HEAD", systemImage: "arrow.triangle.branch")
                            Text(verbatim: root).font(.caption).foregroundStyle(.secondary).textSelection(.enabled)
                            Text("\(Set(changes.files.map(\.path)).count) changed files").font(.subheadline)
                            GitCounts(files: changes.files)
                        }
                        if changes.files.isEmpty {
                            Text("No Git changes").foregroundStyle(.secondary)
                        }
                        ForEach(["staged", "unstaged", "untracked"], id: \.self) { section in
                            let files = changes.files.filter { $0.section == section }
                            if !files.isEmpty {
                                Section(section.capitalized) {
                                    ForEach(Array(files.enumerated()), id: \.offset) { _, file in
                                        NavigationLink {
                                            GitDiffView(agent: agent, root: root, file: file, drafted: drafted)
                                        } label: {
                                            VStack(alignment: .leading, spacing: 4) {
                                                Text(verbatim: file.path).font(.callout)
                                                if let old = file.oldPath {
                                                    Text("From \(old)").font(.caption).foregroundStyle(.secondary)
                                                }
                                                HStack {
                                                    Text(file.statusLabel).foregroundStyle(.secondary)
                                                    Spacer()
                                                    GitCounts(files: [file])
                                                }.font(.caption)
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        if changes.truncated {
                            Text("Some files are not shown. Counts cover the listed entries. Review the remaining changes on the machine.")
                                .font(.footnote).foregroundStyle(.orange)
                        }
                    } else {
                        ContentUnavailableView("Not a Git checkout", systemImage: "folder", description: Text("This agent’s current folder is not in a Git repository."))
                    }
                }
            }
            .overlay { if loading && changes == nil { ProgressView("Loading changes…") } }
            .navigationTitle("Git changes")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button("Done") { dismiss() } }
                ToolbarItem(placement: .primaryAction) {
                    Button("Refresh", systemImage: "arrow.clockwise") { Task { await refresh() } }.disabled(loading)
                }
            }
            .refreshable { await refresh() }
            .task { await refresh() }
        }
    }

    private func refresh() async {
        guard !loading else { return }
        loading = true
        error = nil
        defer { loading = false }
        do { changes = try await agent.gitChanges() }
        catch { changes = nil; self.error = AgentModel.message(for: error) }
    }
}

private struct GitCounts: View {
    let files: [GitFile]

    var body: some View {
        HStack(spacing: 8) {
            Text("+\(files.reduce(UInt64(0)) { $0 + UInt64($1.additions ?? 0) })").foregroundStyle(.green)
            Text("−\(files.reduce(UInt64(0)) { $0 + UInt64($1.deletions ?? 0) })").foregroundStyle(.red)
            if files.contains(where: { $0.additions == nil || $0.deletions == nil }) {
                Text("Text counts unavailable for some files").foregroundStyle(.secondary)
            }
        }.monospacedDigit()
    }
}

private struct GitDiffView: View {
    let agent: AgentModel
    let root: String
    let file: GitFile
    let drafted: () -> Void
    @State private var diff: GitDiff?
    @State private var error: String?
    @State private var loading = false
    @AppStorage("gitWrapLines") private var wraps = true

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                Text(verbatim: file.path).font(.headline).textSelection(.enabled)
                Text("\(file.section.capitalized) · \(file.statusLabel)").font(.subheadline).foregroundStyle(.secondary)
                if let error { Text(error).foregroundStyle(.red) }
                if let diff {
                    let patch = GitPatch(diff)
                    if diff.patch.isEmpty {
                        Text("No text diff. The file may have changed since the list was loaded.").foregroundStyle(.secondary)
                    }
                    if !patch.header.isEmpty { code(patch.header) }
                    ForEach(patch.hunks) { hunk in
                        VStack(alignment: .leading, spacing: 8) {
                            code(hunk.text)
                            Button("Draft feedback", systemImage: "text.bubble") {
                                if agent.draftGitFeedback(hunk.feedback(root: root, file: file)) { drafted() }
                                else { error = agent.promptError ?? "Feedback cannot be added while the composer is busy or answering a question." }
                            }
                            .buttonStyle(.bordered)
                            .disabled(!hunk.complete || !agent.canDraftGitFeedback)
                        }
                    }
                    if diff.truncated {
                        Text("Diff preview is incomplete. Only complete hunks can be selected. Review the rest on the machine.")
                            .font(.footnote).foregroundStyle(.orange)
                    }
                }
                if loading { ProgressView("Loading diff…") }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding()
        }
        .navigationTitle("Diff")
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            ToolbarItem(placement: .primaryAction) {
                Toggle(isOn: $wraps) { Label("Wrap lines", image: "WrapLines") }.toggleStyle(.button)
            }
            ToolbarItem(placement: .primaryAction) {
                Button("Refresh", systemImage: "arrow.clockwise") { Task { await refresh() } }.disabled(loading)
            }
        }
        .task { await refresh() }
    }

    @ViewBuilder private func code(_ text: String) -> some View {
        if wraps { lines(text) }
        else {
            ScrollView(.horizontal) { lines(text).fixedSize(horizontal: true, vertical: false) }
        }
    }

    private func lines(_ text: String) -> some View {
        LazyVStack(alignment: .leading, spacing: 0) {
            ForEach(Array(text.components(separatedBy: "\n").enumerated()), id: \.offset) { _, line in
                Text(verbatim: line.isEmpty ? " " : String(line))
                    .font(.system(.caption, design: .monospaced))
                    .foregroundStyle(line.hasPrefix("+") ? Color.green : line.hasPrefix("-") ? Color.red : Color.primary)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .background(line.hasPrefix("@@") ? Color.accentColor.opacity(0.12) : Color.clear)
            }
        }.textSelection(.enabled)
    }

    private func refresh() async {
        guard !loading else { return }
        loading = true
        diff = nil
        error = nil
        defer { loading = false }
        do { diff = try await agent.gitDiff(root: root, file: file) }
        catch { self.error = AgentModel.message(for: error) }
    }
}

private extension GitFile {
    var statusLabel: String {
        switch status.first {
        case "A": "Added"
        case "D": "Deleted"
        case "R": "Renamed"
        case "C": "Copied"
        case "T": "Type changed"
        case "U": "Conflict"
        case "?": "Untracked"
        default: "Modified"
        }
    }
}
