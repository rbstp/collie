import CollieCore
import SwiftUI
import UIKit

enum GitFeedback {
    static func selectedText(in text: String, range: NSRange) -> String {
        let source = text as NSString
        guard range.location != NSNotFound, range.length > 0,
              range.location >= 0, range.location <= source.length,
              range.length <= source.length - range.location else { return "" }
        return source.substring(with: range)
    }

    static func draft(file: GitFile, selection: String, root: String? = nil) -> String {
        let heading = "Feedback for \(file.path)\(root.map { " in \($0)" } ?? ""): "
        guard !selection.isEmpty else { return heading }
        var fence = "```"
        while selection.contains(fence) { fence += "`" }
        return heading + "\n\n\(fence)diff\n\(selection)\(selection.hasSuffix("\n") ? "" : "\n")\(fence)\n\n"
    }
}

struct GitComment: Identifiable {
    let id = UUID()
    let root: String
    let file: GitFile
    let selection: String
    var anchor: GitDiffAnchor?
    var text = ""

    func message(checkout: String?) -> String {
        GitFeedback.draft(file: file, selection: selection, root: root == checkout ? nil : root)
            + text.trimmingCharacters(in: .whitespacesAndNewlines)
    }
}

@MainActor
@Observable
final class GitReview {
    var comments: [GitComment] = []
    var root: String?
    private(set) var sending = false
    private(set) var error: String?

    var ready: Bool { !comments.isEmpty && comments.allSatisfy { !$0.text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty } }

    @discardableResult
    func add(root: String, file: GitFile, selection: String, anchor: GitDiffAnchor? = nil) -> UUID? {
        guard !sending else { return nil }
        let comment = GitComment(root: root, file: file, selection: selection, anchor: anchor)
        comments.append(comment)
        return comment.id
    }

    func text(for id: UUID) -> Binding<String> {
        Binding(
            get: { self.comments.first { $0.id == id }?.text ?? "" },
            set: { text in
                guard !self.sending, let index = self.comments.firstIndex(where: { $0.id == id }) else { return }
                self.comments[index].text = text
            }
        )
    }

    func remove(id: UUID) {
        guard !sending else { return }
        comments.removeAll { $0.id == id }
    }

    func send(using agent: AgentModel) async -> Bool {
        guard !sending, ready else { return false }
        sending = true
        error = nil
        defer { sending = false }
        let pending = comments
        let checkout: String?
        do {
            let changes = try await agent.gitChanges()
            checkout = changes.root
            let roots = Set(changes.worktrees.map(\.root) + [checkout].compactMap { $0 })
            guard pending.allSatisfy({ roots.contains($0.root) }) else {
                error = "The agent’s checkout changed. Review the feedback before sending."
                return false
            }
        } catch {
            self.error = AgentModel.message(for: error)
            return false
        }
        if let error = await agent.sendGitFeedback(pending.map { $0.message(checkout: checkout) }.joined(separator: "\n\n")) {
            self.error = error
            return false
        }
        let sent = Set(pending.map(\.id))
        comments.removeAll { sent.contains($0.id) }
        return true
    }
}

struct GitChangesView: View {
    let agent: AgentModel
    let sent: () -> Void
    @Bindable var review: GitReview
    @Environment(\.dismiss) private var dismiss
    @State private var changes: GitChanges?
    @State private var error: String?
    @State private var loading = false

    var body: some View {
        NavigationStack {
            List {
                if !review.comments.isEmpty {
                    Section("Feedback (\(review.comments.count))") {
                        ForEach(review.comments) { comment in
                            GitCommentEditor(comment: comment, text: review.text(for: comment.id)) { review.remove(id: comment.id) }
                        }
                        .disabled(review.sending)
                    }
                }
                if let error { Text(error).foregroundStyle(.red) }
                if let changes {
                    if let root = changes.root {
                        Section {
                            if changes.worktrees.count > 1 {
                                Picker(selection: Binding(get: { root }, set: { review.root = $0; Task { await refresh() } })) {
                                    ForEach(changes.worktrees, id: \.root) { worktree in
                                        Text(verbatim: worktree.branch ?? URL(fileURLWithPath: worktree.root).lastPathComponent).tag(worktree.root)
                                    }
                                } label: {
                                    Label("Branch", systemImage: "arrow.triangle.branch")
                                }
                                .disabled(loading)
                            } else {
                                Label(changes.branch ?? "Detached HEAD", systemImage: "arrow.triangle.branch")
                            }
                            Text(verbatim: root).font(.caption).foregroundStyle(.secondary).textSelection(.enabled)
                            Text("\(Set(changes.files.map(\.path)).count) changed files").font(.subheadline)
                            GitCounts(files: changes.files)
                        }
                        if changes.files.isEmpty {
                            Text("No Git changes").foregroundStyle(.secondary)
                        }
                        ForEach(["committed", "staged", "unstaged", "untracked"], id: \.self) { section in
                            let files = changes.files.filter { $0.section == section }
                            if !files.isEmpty {
                                Section(section.capitalized) {
                                    ForEach(Array(files.enumerated()), id: \.offset) { _, file in
                                        NavigationLink {
                                            GitDiffView(agent: agent, root: root, file: file, sent: sent, review: review)
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
            .safeAreaInset(edge: .bottom) { GitReviewBar(agent: agent, review: review, sent: sent) }
            .overlay { if loading && changes == nil { ProgressView("Loading changes…") } }
            .navigationTitle("Git changes")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button("Done") { dismiss() }.disabled(review.sending) }
                ToolbarItem(placement: .primaryAction) {
                    Button("Refresh", systemImage: "arrow.clockwise") { Task { await refresh() } }.disabled(loading)
                }
            }
            .refreshable { await refresh() }
            .task { await refresh() }
        }
        .interactiveDismissDisabled(review.sending)
    }

    private func refresh() async {
        guard !loading else { return }
        loading = true
        error = nil
        defer { loading = false }
        do {
            changes = try await agent.gitChanges(root: review.root)
            if changes?.root != review.root { review.root = nil }
        }
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

struct GitDiffView: View {
    let agent: AgentModel
    let root: String
    let file: GitFile
    let sent: () -> Void
    @Bindable var review: GitReview
    @FocusState private var editing: UUID?
    @State private var collapsed = false
    @State private var diff: GitDiff?
    @State private var rows: [GitDiffRow] = []
    @State private var error: String?
    @State private var loading = false
    @AppStorage("gitWrapLines") private var wraps = true

    var body: some View {
        ScrollViewReader { scroll in
            GeometryReader { geometry in
                VStack(spacing: 0) {
                    header
                    ScrollView {
                        VStack(alignment: .leading, spacing: 0) {
                            if let error { Text(error).foregroundStyle(.red).padding() }
                            if let diff, !collapsed {
                                let anchors = Set(comments.compactMap { $0.anchor?.patch == diff.patch ? $0.anchor?.row : nil })
                                let blocks = GitDiffBlock.split(rows, after: anchors)
                                let digits = String(rows.reduce(1) { max($0, $1.oldLine ?? 0, $1.newLine ?? 0) }).count
                                if rows.isEmpty {
                                    Text("No text diff. The file may be empty, renamed, or have changed since the list was loaded.")
                                        .foregroundStyle(.secondary).padding()
                                }
                                ForEach(blocks) { block in
                                    code(block, patch: diff.patch, width: geometry.size.width, digits: digits)
                                    ForEach(comments.filter { $0.anchor == GitDiffAnchor(patch: diff.patch, row: block.lastRow) }) { comment in
                                        editor(comment)
                                    }
                                }
                                if diff.truncated {
                                    Text("Diff preview is incomplete. Review the rest on the machine.")
                                        .font(.footnote).foregroundStyle(.orange).padding()
                                }
                                ForEach(comments.filter { comment in comment.anchor?.patch != diff.patch || !rows.contains(where: { $0.id == comment.anchor?.row }) }) { comment in
                                    editor(comment)
                                }
                            }
                            if diff == nil, !collapsed {
                                ForEach(comments) { comment in editor(comment) }
                            }
                            if loading { ProgressView("Loading diff…").padding() }
                        }
                        .frame(maxWidth: .infinity, alignment: .leading)
                    }
                    .onChange(of: editing) { _, id in
                        if let id { withAnimation { scroll.scrollTo(id, anchor: .bottom) } }
                    }
                    GitReviewBar(agent: agent, review: review, sent: sent)
                }
                .onChange(of: geometry.size.height) {
                    if let editing { withAnimation { scroll.scrollTo(editing, anchor: .bottom) } }
                }
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
    }

    private var comments: [GitComment] {
        review.comments.filter { $0.root == root && $0.file.path == file.path && $0.file.section == file.section }
    }

    private var header: some View {
        HStack(alignment: .top, spacing: 12) {
            Button {
                collapsed.toggle()
                if collapsed { editing = nil }
            } label: {
                Image(systemName: collapsed ? "chevron.right" : "chevron.down")
                    .frame(minWidth: 24, minHeight: 32)
            }
            .accessibilityLabel(collapsed ? "Expand diff" : "Collapse diff")
            VStack(alignment: .leading, spacing: 4) {
                Text(verbatim: file.path).font(.subheadline.weight(.semibold)).textSelection(.enabled)
                Text("\(file.section.capitalized) · \(file.statusLabel)").font(.caption).foregroundStyle(.secondary)
                if let old = file.oldPath { Text("From \(old)").font(.caption).foregroundStyle(.secondary) }
            }
            Spacer(minLength: 0)
            Button("File feedback", systemImage: "text.bubble") {
                collapsed = false
                editing = review.add(root: root, file: file, selection: "")
            }
            .labelStyle(.iconOnly)
            .buttonStyle(.glass)
            .disabled(review.sending)
        }
        .padding(12)
        .glassEffect(.regular, in: .rect(cornerRadius: 16))
        .padding(.horizontal, 8)
        .padding(.bottom, 8)
        .dynamicTypeSize(...DynamicTypeSize.xxxLarge)
    }

    private func editor(_ comment: GitComment) -> some View {
        GitCommentEditor(comment: comment, text: review.text(for: comment.id), editing: $editing) {
            if editing == comment.id { editing = nil }
            review.remove(id: comment.id)
        }
        .padding(12)
        .id(comment.id)
        .disabled(review.sending)
    }

    @ViewBuilder private func code(_ block: GitDiffBlock, patch: String, width: CGFloat, digits: Int) -> some View {
        let feedback: ((String, Int) -> Void)? = review.sending ? nil : { text, row in
            editing = review.add(root: root, file: file, selection: text, anchor: GitDiffAnchor(patch: patch, row: row))
        }
        if wraps { GitDiffText(block: block, wraps: true, minimumWidth: width, digits: digits, feedback: feedback) }
        else {
            ScrollView(.horizontal) { GitDiffText(block: block, wraps: false, minimumWidth: width, digits: digits, feedback: feedback) }
        }
    }

    private func refresh() async {
        guard !loading else { return }
        loading = true
        diff = nil
        rows = []
        error = nil
        defer { loading = false }
        do {
            let loaded = try await agent.gitDiff(root: root, file: file)
            rows = GitDiffRow.parse(loaded.patch)
            diff = loaded
        } catch { self.error = AgentModel.message(for: error) }
    }
}

private struct GitCommentEditor: View {
    let comment: GitComment
    @Binding var text: String
    var editing: FocusState<UUID?>.Binding?
    let remove: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack {
                Text(verbatim: comment.file.path).font(.caption.weight(.semibold)).fixedSize(horizontal: false, vertical: true)
                Spacer()
                Button("Remove feedback", systemImage: "trash", role: .destructive, action: remove)
                    .labelStyle(.iconOnly).buttonStyle(.glass)
            }
            if !comment.selection.isEmpty {
                Text(verbatim: comment.selection).font(.system(.caption, design: .monospaced)).foregroundStyle(.secondary)
            }
            if let editing {
                field.focused(editing, equals: comment.id)
            } else { field }
        }
        .padding(12)
        .glassEffect(.regular, in: .rect(cornerRadius: 16))
    }

    private var field: some View {
        TextField("Feedback", text: $text, axis: .vertical).lineLimit(2...8)
    }
}

private struct GitReviewBar: View {
    let agent: AgentModel
    let review: GitReview
    let sent: () -> Void

    var body: some View {
        VStack(spacing: 8) {
            if let error = review.error {
                Text(error).font(.footnote).foregroundStyle(.red)
                    .padding(8).glassEffect(.regular, in: .rect(cornerRadius: 12))
            }
            if !agent.canSendGitFeedback && !review.sending {
                Text("Feedback stays here until the agent can accept a prompt.").font(.footnote).foregroundStyle(.secondary)
                    .padding(8).glassEffect(.regular, in: .rect(cornerRadius: 12))
            }
            Button {
                Task { if await review.send(using: agent) { sent() } }
            } label: {
                if review.sending { ProgressView() }
                else { Label("Send feedback (\(review.comments.count))", systemImage: "text.bubble") }
            }
            .buttonStyle(.glassProminent)
            .controlSize(.large)
            .disabled(review.sending || !review.ready || !agent.canSendGitFeedback)
        }
        .padding(12)
        .frame(maxWidth: .infinity)
        .dynamicTypeSize(...DynamicTypeSize.xxxLarge)
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
