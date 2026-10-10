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

    static func draft(file: GitFile, selection: String) -> String {
        let heading = "Feedback for \(file.path): "
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
    var text = ""

    var message: String { GitFeedback.draft(file: file, selection: selection) + text.trimmingCharacters(in: .whitespacesAndNewlines) }
}

@MainActor
@Observable
final class GitReview {
    var comments: [GitComment] = []
    private(set) var sending = false
    private(set) var error: String?

    var ready: Bool { !comments.isEmpty && comments.allSatisfy { !$0.text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty } }

    @discardableResult
    func add(root: String, file: GitFile, selection: String) -> UUID? {
        guard !sending else { return nil }
        let comment = GitComment(root: root, file: file, selection: selection)
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
        do {
            let checkout = try await agent.gitChanges().root
            guard pending.allSatisfy({ $0.root == checkout }) else {
                error = "The agent’s checkout changed. Review the feedback before sending."
                return false
            }
        } catch {
            self.error = AgentModel.message(for: error)
            return false
        }
        if let error = await agent.sendGitFeedback(pending.map(\.message).joined(separator: "\n\n")) {
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
    let sent: () -> Void
    @Bindable var review: GitReview
    @FocusState private var editing: UUID?
    @State private var selection = NSRange(location: 0, length: 0)
    @State private var diff: GitDiff?
    @State private var error: String?
    @State private var loading = false
    @AppStorage("gitWrapLines") private var wraps = true

    var body: some View {
        ScrollViewReader { scroll in
            ScrollView {
                VStack(alignment: .leading, spacing: 16) {
                    Text(verbatim: file.path).font(.headline).textSelection(.enabled)
                    Text("\(file.section.capitalized) · \(file.statusLabel)").font(.subheadline).foregroundStyle(.secondary)
                    if let error { Text(error).foregroundStyle(.red) }
                    if let diff {
                        if diff.patch.isEmpty {
                            Text("No text diff. The file may have changed since the list was loaded.").foregroundStyle(.secondary)
                        }
                        code(diff.patch)
                        if diff.truncated {
                            Text("Diff preview is incomplete. Review the rest on the machine.")
                                .font(.footnote).foregroundStyle(.orange)
                        }
                    }
                    if loading { ProgressView("Loading diff…") }
                    ForEach(review.comments) { comment in
                        if comment.root == root && comment.file.path == file.path && comment.file.section == file.section {
                            GitCommentEditor(comment: comment, text: review.text(for: comment.id), editing: $editing) {
                                if editing == comment.id { editing = nil }
                                review.remove(id: comment.id)
                            }
                                .id(comment.id)
                                .disabled(review.sending)
                        }
                    }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding()
            }
            .safeAreaInset(edge: .bottom) {
                GitReviewBar(agent: agent, review: review, sent: sent, add: diff.map { diff in
                    {
                        let text = GitFeedback.selectedText(in: diff.patch, range: selection)
                        editing = review.add(root: root, file: file, selection: text)
                        selection = NSRange(location: 0, length: 0)
                    }
                })
            }
            .onChange(of: editing) { _, id in
                if let id { withAnimation { scroll.scrollTo(id, anchor: .bottom) } }
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

    @ViewBuilder private func code(_ text: String) -> some View {
        if wraps { GitDiffText(text: text, wraps: true, selection: $selection) }
        else {
            ScrollView(.horizontal) { GitDiffText(text: text, wraps: false, selection: $selection) }
        }
    }

    private func refresh() async {
        guard !loading else { return }
        loading = true
        selection = NSRange(location: 0, length: 0)
        diff = nil
        error = nil
        defer { loading = false }
        do { diff = try await agent.gitDiff(root: root, file: file) }
        catch { self.error = AgentModel.message(for: error) }
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
                Text(verbatim: comment.file.path).font(.caption.weight(.semibold))
                Spacer()
                Button("Remove feedback", systemImage: "trash", role: .destructive, action: remove).labelStyle(.iconOnly)
            }
            if !comment.selection.isEmpty {
                Text(verbatim: comment.selection).font(.system(.caption, design: .monospaced)).foregroundStyle(.secondary)
            }
            if let editing {
                field.focused(editing, equals: comment.id)
            } else { field }
        }
        .padding(12)
        .background(.fill.tertiary, in: RoundedRectangle(cornerRadius: 12))
    }

    private var field: some View {
        TextField("Feedback", text: $text, axis: .vertical).lineLimit(2...8)
    }
}

private struct GitReviewBar: View {
    let agent: AgentModel
    let review: GitReview
    let sent: () -> Void
    var add: (() -> Void)?

    var body: some View {
        VStack(spacing: 8) {
            if let error = review.error { Text(error).font(.footnote).foregroundStyle(.red) }
            if !agent.canSendGitFeedback && !review.sending {
                Text("Feedback stays here until the agent can accept a prompt.").font(.footnote).foregroundStyle(.secondary)
            }
            HStack {
                if let add {
                    Button("Add feedback", systemImage: "text.bubble", action: add)
                        .buttonStyle(.bordered)
                        .disabled(review.sending)
                }
                Button {
                    Task { if await review.send(using: agent) { sent() } }
                } label: {
                    if review.sending { ProgressView() }
                    else { Text("Send feedback (\(review.comments.count))") }
                }
                .buttonStyle(.borderedProminent)
                .disabled(review.sending || !review.ready || !agent.canSendGitFeedback)
            }
        }
        .padding()
        .frame(maxWidth: .infinity)
        .background(.bar)
    }
}

struct GitDiffText: UIViewRepresentable {
    let text: String
    let wraps: Bool
    @Binding var selection: NSRange
    @Environment(\.dynamicTypeSize) private var dynamicTypeSize

    func makeUIView(context: Context) -> UITextView {
        let view = UITextView()
        view.isEditable = false
        view.isSelectable = true
        view.isScrollEnabled = false
        view.backgroundColor = .clear
        view.textContainerInset = .zero
        view.textContainer.lineFragmentPadding = 0
        view.textContainer.widthTracksTextView = false
        view.delegate = context.coordinator
        return view
    }

    func updateUIView(_ view: UITextView, context: Context) {
        context.coordinator.selection = $selection
        context.coordinator.updating = true
        defer { context.coordinator.updating = false }
        let font = UIFont.monospacedSystemFont(ofSize: UIFont.preferredFont(forTextStyle: .caption1).pointSize, weight: .regular)
        if view.text != text || view.font != font {
            let styled = NSMutableAttributedString(string: text, attributes: [.font: font, .foregroundColor: UIColor.label])
            let source = text as NSString
            var offset = 0
            while offset < source.length {
                let range = source.lineRange(for: NSRange(location: offset, length: 0))
                let line = source.substring(with: range)
                if line.hasPrefix("+") { styled.addAttribute(.foregroundColor, value: UIColor.systemGreen, range: range) }
                else if line.hasPrefix("-") { styled.addAttribute(.foregroundColor, value: UIColor.systemRed, range: range) }
                else if line.hasPrefix("@@") { styled.addAttribute(.backgroundColor, value: UIColor.tintColor.withAlphaComponent(0.12), range: range) }
                offset = NSMaxRange(range)
            }
            view.attributedText = styled
        }
        view.textContainer.lineBreakMode = wraps ? .byWordWrapping : .byClipping
        if view.selectedRange != selection { view.selectedRange = selection }
    }

    func sizeThatFits(_ proposal: ProposedViewSize, uiView: UITextView, context: Context) -> CGSize? {
        let width = wraps ? (proposal.width ?? 300) : max(1, ceil(uiView.attributedText.size().width) + 1)
        uiView.textContainer.size = CGSize(width: width, height: .greatestFiniteMagnitude)
        return CGSize(width: width, height: ceil(uiView.sizeThatFits(CGSize(width: width, height: .greatestFiniteMagnitude)).height))
    }

    func makeCoordinator() -> Coordinator { Coordinator(selection: $selection) }

    final class Coordinator: NSObject, UITextViewDelegate {
        var selection: Binding<NSRange>
        var updating = false

        init(selection: Binding<NSRange>) { self.selection = selection }

        func textViewDidChangeSelection(_ textView: UITextView) {
            guard !updating, selection.wrappedValue != textView.selectedRange else { return }
            selection.wrappedValue = textView.selectedRange
        }
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
