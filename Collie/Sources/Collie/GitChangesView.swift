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
        let place = file.pullRequest.map { " in pull request #\($0)" } ?? root.map { " in \($0)" } ?? ""
        let heading = "Feedback for \(file.path)\(place): "
        guard !selection.isEmpty else { return heading }
        return heading + "\n\n\(CodeFence.block(selection, info: "diff"))\n\n"
    }
}

enum GitCleanup {
    /// collied's default branch falls back to main, then master; the prompt guards an origin/HEAD naming another.
    static func selectable(_ worktree: GitWorktree, checkout: String?) -> Bool {
        worktree.root != checkout && worktree.branch != "main" && worktree.branch != "master"
    }

    static func prompt(_ worktrees: [GitWorktree]) -> String {
        let list = worktrees.map { "- \(quoted($0.root)) (\($0.branch.map { "branch \(quoted($0))" } ?? "detached HEAD"))" }
        return """
        Clean up these Git worktrees (paths and branches are JSON strings):
        \(list.joined(separator: "\n"))

        Check that each one is not in use: no agent or process is working in it, it has no uncommitted or untracked changes and no unpushed commits, and its pull request is merged or closed, or it has none. If it has no pull request, also check that nothing in it was created or changed in the last 24 hours. Remove only the worktrees that pass, with their local branches. Never remove the main worktree or the default branch. Report which worktrees you kept and why.
        """
    }

    /// Paths and branch names may hold U+2028 or U+2029, which would start a line of their own in the prompt.
    static func quoted(_ text: String) -> String {
        let escaped = text.unicodeScalars.map { scalar -> String in
            switch scalar {
            case "\"", "\\": return "\\" + String(scalar)
            default:
                switch scalar.properties.generalCategory {
                case .control, .format, .lineSeparator, .paragraphSeparator:
                    return String(scalar).utf16.map { String(format: "\\u%04X", $0) }.joined()
                default: return String(scalar)
                }
            }
        }
        return "\"" + escaped.joined() + "\""
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
    var selection: Set<String>?
    private(set) var sending = false
    private(set) var error: String?
    private(set) var viewed: GitViewed
    private let viewedFile: URL?

    init(viewedFile: URL? = nil) {
        self.viewedFile = viewedFile
        viewed = GitViewed.load(from: viewedFile)
    }

    func isViewed(route: AgentRoute, root: String, file: GitFile) -> Bool {
        GitViewed.mark(route: route, root: root, file: file).map { viewed.marks[$0] != nil } ?? false
    }

    func ordered(_ files: [GitFile], route: AgentRoute, root: String) -> [(file: GitFile, viewed: Bool)] {
        let entries = files.map { (file: $0, viewed: isViewed(route: route, root: root, file: $0)) }
        return entries.filter { !$0.viewed } + entries.filter(\.viewed)
    }

    func setViewed(_ on: Bool, route: AgentRoute, root: String, file: GitFile) {
        guard let mark = GitViewed.mark(route: route, root: root, file: file) else { return }
        var saved = viewedFile == nil ? viewed : GitViewed.load(from: viewedFile)
        saved.set(mark, viewed: on, nowMs: Date().unixMs)
        saved.save(to: viewedFile)
        viewed = saved
    }

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
            // A pull request's files carry the agent's checkout as their root: another checkout may be another repository.
            guard pending.allSatisfy({ $0.file.pullRequest == nil ? roots.contains($0.root) : $0.root == checkout }) else {
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

    func select(_ on: Bool) {
        guard !sending else { return }
        selection = on ? [] : nil
        error = nil
    }

    func cleanUp(_ worktrees: [GitWorktree], checkout: String, using agent: AgentModel) async -> Bool {
        guard !sending, !worktrees.isEmpty else { return false }
        sending = true
        error = nil
        defer { sending = false }
        do {
            guard try await agent.gitChanges().root == checkout else {
                error = "The agent’s checkout changed. Refresh the worktrees before cleaning up."
                return false
            }
        } catch {
            self.error = AgentModel.message(for: error)
            return false
        }
        if let error = await agent.sendGitFeedback(
            GitCleanup.prompt(worktrees), noun: "a cleanup request", tooLong: "The cleanup request is too long. Select fewer worktrees."
        ) {
            self.error = error
            return false
        }
        selection = nil
        return true
    }
}

/// Which agents work in which worktree: each goes to the deepest worktree holding its folder.
enum GitWorktreeAgents {
    static func place(_ agents: [AgentSummary], in roots: [String]) -> [String: [AgentSummary]] {
        var placed: [String: [AgentSummary]] = [:]
        for agent in agents {
            guard let cwd = agent.cwd,
                  let root = roots.filter({ cwd == $0 || cwd.hasPrefix($0.hasSuffix("/") ? $0 : $0 + "/") })
                      .max(by: { $0.count < $1.count }) else { continue }
            placed[root, default: []].append(agent)
        }
        return placed
    }
}

struct GitChangesView: View {
    let agent: AgentModel
    let sent: () -> Void
    @Bindable var review: GitReview
    var open: ((AgentRoute) -> Void)?
    @Environment(\.dismiss) private var dismiss
    @State private var overview: GitChanges?
    @State private var agents: [String: [AgentSummary]] = [:]
    @State private var error: String?
    @State private var loading = false

    var body: some View {
        NavigationStack {
            List(selection: selection) {
                if !selecting { GitFeedbackSection(review: review) }
                if let error { Text(error).font(.footnote).foregroundStyle(.red) }
                if let overview {
                    if let checkout = overview.root {
                        Section {
                            ForEach(overview.worktrees, id: \.root) { worktree in
                                let here = agents[worktree.root] ?? []
                                let row = GitWorktreeRow(worktree: worktree, current: worktree.root == checkout, agents: here)
                                Group {
                                    if selecting { row } else {
                                        NavigationLink {
                                            GitWorktreeView(
                                                agent: agent, root: worktree.root, branch: worktree.branch, pullRequest: worktree.pullRequest,
                                                local: true, agents: here, sent: sent, review: review, open: open
                                            )
                                        } label: { row }
                                    }
                                }
                                .listRowInsets(GitRow.insets)
                                .selectionDisabled(!GitCleanup.selectable(worktree, checkout: checkout))
                            }
                        } header: {
                            GitSectionHeader(title: "Worktrees")
                        }
                        if !overview.pullRequests.isEmpty && !selecting {
                            Section {
                                ForEach(overview.pullRequests, id: \.pullRequest.number) { pull in
                                    NavigationLink {
                                        GitWorktreeView(
                                            agent: agent, root: checkout, branch: pull.branch, pullRequest: pull.pullRequest,
                                            local: false, agents: [], sent: sent, review: review, open: open
                                        )
                                    } label: {
                                        GitPullRequestRow(pull: pull)
                                    }
                                    .listRowInsets(GitRow.insets)
                                }
                            } header: {
                                GitSectionHeader(title: "Pull requests without a worktree")
                            }
                        }
                        if overview.truncated {
                            Text("Some pull requests are not shown.").font(.caption).foregroundStyle(.orange)
                        }
                    } else {
                        ContentUnavailableView("Not a Git checkout", systemImage: "folder", description: Text("This agent’s current folder is not in a Git repository."))
                    }
                }
            }
            .gitCompactList()
            .environment(\.editMode, .constant(selecting ? .active : .inactive))
            .safeAreaInset(edge: .bottom) {
                GitReviewBar(agent: agent, review: review, sent: sent, cleanup: selecting ? cleanup : nil, checkout: overview?.root)
            }
            .overlay { if loading && overview == nil { ProgressView("Loading worktrees…") } }
            .navigationTitle("Git changes")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button("Done") { dismiss() }.disabled(review.sending) }
                if selecting {
                    ToolbarItem(placement: .primaryAction) { Button("Cancel") { review.select(false) }.disabled(review.sending) }
                } else {
                    if let overview, overview.worktrees.contains(where: { GitCleanup.selectable($0, checkout: overview.root) }) {
                        ToolbarItem(placement: .primaryAction) { Button("Select") { review.select(true) }.disabled(review.sending) }
                    }
                    ToolbarItem(placement: .primaryAction) {
                        Button("Refresh", systemImage: "arrow.clockwise") { Task { await refresh() } }.disabled(loading)
                    }
                }
            }
            .refreshable { await refresh() }
            .task { await refresh() }
        }
        .interactiveDismissDisabled(review.sending)
        .onAppear { if review.selection != nil { review.select(false) } }
    }

    private var selecting: Bool { review.selection != nil }

    private var selection: Binding<Set<String>> {
        Binding(get: { review.selection ?? [] }, set: { if review.selection != nil && !review.sending { review.selection = $0 } })
    }

    private var cleanup: [GitWorktree] {
        guard let overview, let selected = review.selection else { return [] }
        return overview.worktrees.filter { selected.contains($0.root) && GitCleanup.selectable($0, checkout: overview.root) }
    }

    private func refresh() async {
        guard !loading else { return }
        loading = true
        error = nil
        defer { loading = false }
        do {
            async let machine = agent.machineAgents()
            let loaded = try await agent.gitWorktrees()
            overview = loaded
            agents = GitWorktreeAgents.place(await machine, in: loaded.worktrees.map(\.root))
        } catch { overview = nil; self.error = AgentModel.message(for: error) }
    }
}

private struct GitWorktreeRow: View {
    let worktree: GitWorktree
    let current: Bool
    let agents: [AgentSummary]

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            HStack(spacing: 6) {
                Text(verbatim: worktree.branch ?? URL(fileURLWithPath: worktree.root).lastPathComponent)
                    .font(.subheadline.weight(.semibold)).lineLimit(1)
                if current {
                    Image(systemName: "location.fill").font(.caption2).foregroundStyle(.tint)
                        .accessibilityLabel("This session’s checkout")
                }
                Spacer(minLength: 4)
                if !agents.isEmpty {
                    HStack(spacing: 2) {
                        ForEach(Array(agents.prefix(4).enumerated()), id: \.offset) { _, agent in
                            Circle().fill(agent.status.color).frame(width: 6, height: 6)
                        }
                        Text("\(agents.count)").font(.caption2).foregroundStyle(.secondary).monospacedDigit()
                    }
                    .accessibilityElement(children: .ignore)
                    .accessibilityLabel(agents.count == 1 ? "1 agent" : "\(agents.count) agents")
                }
                if let pullRequest = worktree.pullRequest {
                    PullRequestBadge(pullRequest: pullRequest, watch: nil, compact: true)
                }
            }
            if let subject = worktree.subject {
                HStack(spacing: 4) {
                    Text(verbatim: subject).lineLimit(1)
                    if let at = worktree.committedAtMs {
                        Spacer(minLength: 4)
                        Text(Elapsed.compact(sinceMs: at, now: Date())).monospacedDigit()
                    }
                }
                .font(.caption).foregroundStyle(.secondary)
            }
            GitWorktreeCounts(worktree: worktree)
        }
    }
}

private struct GitWorktreeCounts: View {
    let worktree: GitWorktree

    var body: some View {
        HStack(spacing: 8) {
            if let ahead = worktree.ahead, ahead > 0 {
                Label("\(ahead)", systemImage: "arrow.up").accessibilityLabel("\(ahead) commits ahead")
            }
            if let uncommitted = worktree.uncommitted, uncommitted > 0 {
                Label("\(uncommitted)", systemImage: "pencil").accessibilityLabel("\(uncommitted) uncommitted files")
            }
            if let additions = worktree.additions, let deletions = worktree.deletions, additions + deletions > 0 {
                Text("+\(additions)").foregroundStyle(.green)
                Text("−\(deletions)").foregroundStyle(.red)
            }
        }
        .labelStyle(GitTightLabel())
        .font(.caption2).foregroundStyle(.secondary).monospacedDigit()
    }
}

private struct GitTightLabel: LabelStyle {
    func makeBody(configuration: Configuration) -> some View {
        HStack(spacing: 1) { configuration.icon.imageScale(.small); configuration.title }
    }
}

private struct GitPullRequestRow: View {
    let pull: GitPullRequest

    var body: some View {
        VStack(alignment: .leading, spacing: 2) {
            HStack(spacing: 6) {
                Text(verbatim: pull.branch).font(.subheadline.weight(.semibold)).lineLimit(1)
                Spacer(minLength: 4)
                PullRequestBadge(pullRequest: pull.pullRequest, watch: nil, compact: true)
            }
            Text(verbatim: pull.title).font(.caption).foregroundStyle(.secondary).lineLimit(1)
        }
    }
}

/// One worktree's files, or, with `local` off, only the pull request of a branch whose worktree is gone.
struct GitWorktreeView: View {
    enum Scope: Hashable { case uncommitted, branch, pullRequest }

    let agent: AgentModel
    let root: String
    let branch: String?
    let pullRequest: PullRequestSummary?
    let local: Bool
    let agents: [AgentSummary]
    let sent: () -> Void
    @Bindable var review: GitReview
    var open: ((AgentRoute) -> Void)?
    @State private var scope: Scope?
    @State private var changes: GitChanges?
    @State private var error: String?
    @State private var loading = false

    private var scopes: [Scope] {
        let open = pullRequest.map { $0.state == .open || $0.state == .draft } ?? false
        return (local ? [.uncommitted, .branch] : []) + (open ? [.pullRequest] : [])
    }

    private var selected: Scope { scope ?? scopes.first ?? .uncommitted }

    var body: some View {
        List {
            Section {
                header
                if scopes.count > 1 {
                    Picker("Scope", selection: Binding(get: { selected }, set: { scope = $0 })) {
                        ForEach(scopes, id: \.self) { scope in
                            switch scope {
                            case .uncommitted: Text("Uncommitted").tag(scope)
                            case .branch: Text("Branch").tag(scope)
                            case .pullRequest: Text("Pull request").tag(scope)
                            }
                        }
                    }
                    .pickerStyle(.segmented)
                    .controlSize(.small)
                    .disabled(loading)
                    .onChange(of: scope) { changes = nil; Task { await refresh() } }
                }
            }
            GitFeedbackSection(review: review)
            if let error { Text(error).font(.footnote).foregroundStyle(.red) }
            if let changes, let fileRoot = changes.root {
                if selected == .branch && changes.base == nil {
                    Text("No default branch to compare with. This checkout has no origin/HEAD, main or master.")
                        .font(.footnote).foregroundStyle(.secondary)
                } else if changes.files.isEmpty {
                    Text(selected == .uncommitted ? "No Git changes" : "No changes on this branch").font(.footnote).foregroundStyle(.secondary)
                }
                ForEach(["pull_request", "committed", "staged", "unstaged", "untracked"], id: \.self) { section in
                    let files = review.ordered(changes.files.filter { $0.section == section }, route: agent.route, root: fileRoot)
                    if !files.isEmpty {
                        Section {
                            ForEach(Array(files.enumerated()), id: \.offset) { _, entry in
                                NavigationLink {
                                    GitDiffView(agent: agent, root: fileRoot, file: entry.file, sent: sent, review: review)
                                } label: {
                                    GitFileRow(file: entry.file, viewed: entry.viewed)
                                }
                                .listRowInsets(GitRow.insets)
                            }
                        } header: {
                            GitSectionHeader(title: GitFile.sectionLabel(section))
                        }
                    }
                }
                if changes.truncated {
                    Text("Some files are not shown. Counts cover the listed entries. Review the remaining changes on the machine.")
                        .font(.caption).foregroundStyle(.orange)
                }
            }
        }
        .gitCompactList()
        .safeAreaInset(edge: .bottom) { GitReviewBar(agent: agent, review: review, sent: sent) }
        .overlay { if loading && changes == nil { ProgressView("Loading changes…") } }
        .navigationTitle(branch ?? URL(fileURLWithPath: root).lastPathComponent)
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            ToolbarItem(placement: .primaryAction) {
                Button("Refresh", systemImage: "arrow.clockwise") { Task { await refresh() } }.disabled(loading)
            }
        }
        .refreshable { await refresh() }
        .task { await refresh() }
    }

    private var header: some View {
        VStack(alignment: .leading, spacing: 3) {
            HStack(spacing: 6) {
                Text(verbatim: root).font(.caption2).foregroundStyle(.secondary).lineLimit(1).truncationMode(.head)
                    .textSelection(.enabled)
                Spacer(minLength: 4)
                if let pullRequest { PullRequestBadge(pullRequest: pullRequest, watch: nil, compact: true) }
            }
            if let changes {
                HStack(spacing: 8) {
                    if let base = changes.base, selected != .uncommitted {
                        Text(selected == .pullRequest ? "Into \(base)" : "Since \(base)").lineLimit(1)
                    }
                    let count = Set(changes.files.map(\.path)).count
                    Text(count == 1 ? "1 file" : "\(count) files")
                    GitCounts(files: changes.files)
                }
                .font(.caption).foregroundStyle(.secondary)
            }
            if !agents.isEmpty {
                ScrollView(.horizontal) {
                    HStack(spacing: 6) {
                        ForEach(agents, id: \.terminalId) { summary in
                            Button {
                                open?(AgentRoute(machineId: agent.route.machineId, terminalId: summary.terminalId))
                            } label: {
                                HStack(spacing: 3) {
                                    Circle().fill(summary.status.color).frame(width: 6, height: 6)
                                    Text(verbatim: summary.displayTitle).lineLimit(1)
                                }
                                .font(.caption2)
                            }
                            .buttonStyle(.borderless)
                            .disabled(open == nil)
                            .accessibilityLabel("\(summary.displayTitle), \(summary.status.label)")
                        }
                    }
                }
                .scrollIndicators(.hidden)
            }
        }
    }

    private func refresh() async {
        guard !loading else { return }
        loading = true
        error = nil
        defer { loading = false }
        do {
            let loaded: GitChanges
            if selected == .pullRequest, let number = pullRequest?.number {
                loaded = try await agent.gitPullRequest(number)
            } else {
                loaded = try await agent.gitChanges(root: root, branch: selected == .branch)
                guard loaded.root == root else {
                    changes = nil
                    error = "This worktree is no longer listed. Go back and refresh."
                    return
                }
            }
            changes = loaded
        } catch { changes = nil; self.error = AgentModel.message(for: error) }
    }
}

private struct GitFileRow: View {
    let file: GitFile
    let viewed: Bool

    var body: some View {
        HStack(spacing: 6) {
            Text(verbatim: file.statusMarker)
                .font(.caption2.monospaced().weight(.bold))
                .foregroundStyle(file.statusColor)
                .frame(minWidth: 12)
            VStack(alignment: .leading, spacing: 0) {
                Text(verbatim: file.path).font(.footnote).lineLimit(1).truncationMode(.head)
                if let old = file.oldPath, !viewed {
                    Text("From \(old)").font(.caption2).foregroundStyle(.secondary).lineLimit(1).truncationMode(.head)
                }
            }
            Spacer(minLength: 4)
            if viewed {
                Image(systemName: "checkmark.circle.fill").font(.caption).foregroundStyle(.secondary)
            } else {
                GitCounts(files: [file]).font(.caption2)
            }
        }
        .foregroundStyle(viewed ? .secondary : .primary)
        .accessibilityElement(children: .combine)
        .accessibilityLabel("\(file.path), \(file.statusLabel)\(viewed ? ", viewed" : "")")
    }
}

private struct GitFeedbackSection: View {
    @Bindable var review: GitReview

    var body: some View {
        if !review.comments.isEmpty {
            Section {
                ForEach(review.comments) { comment in
                    GitCommentEditor(comment: comment, text: review.text(for: comment.id)) { review.remove(id: comment.id) }
                        .font(.footnote)
                        .controlSize(.small)
                        .listRowInsets(EdgeInsets(top: 2, leading: 8, bottom: 2, trailing: 8))
                        .listRowBackground(Color.clear)
                }
                .disabled(review.sending)
            } header: {
                GitSectionHeader(title: "Feedback (\(review.comments.count))")
            }
        }
    }
}

private struct GitSectionHeader: View {
    let title: String

    var body: some View {
        Text(title).font(.caption.weight(.semibold)).foregroundStyle(.secondary)
    }
}

private enum GitRow {
    static let insets = EdgeInsets(top: 5, leading: 14, bottom: 5, trailing: 14)
}

private extension View {
    func gitCompactList() -> some View {
        listStyle(.insetGrouped)
            .listSectionSpacing(.compact)
            .environment(\.defaultMinListRowHeight, 32)
    }
}

private struct GitCounts: View {
    let files: [GitFile]

    var body: some View {
        HStack(spacing: 6) {
            Text("+\(files.reduce(UInt64(0)) { $0 + UInt64($1.additions ?? 0) })").foregroundStyle(.green)
            Text("−\(files.reduce(UInt64(0)) { $0 + UInt64($1.deletions ?? 0) })").foregroundStyle(.red)
            if files.contains(where: { $0.additions == nil || $0.deletions == nil }) {
                Text(files.count == 1 ? "counts unavailable" : "some counts unavailable").foregroundStyle(.secondary)
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
    @Environment(\.neutralAccent) private var neutralAccent

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
                    Button { wraps.toggle() } label: { Label(wraps ? "Wrap lines on" : "Wrap lines off", image: "WrapLines") }
                        .tint(wraps ? Color.accentColor : neutralAccent ? .secondary : .primary)
                }
                ToolbarItem(placement: .primaryAction) {
                    Button("Refresh", systemImage: "arrow.clockwise") { Task { await refresh() } }.disabled(loading)
                }
            }
            .onAppear { if viewed.wrappedValue { collapsed = true } }
            .task { await refresh() }
        }
    }

    private var viewed: Binding<Bool> {
        Binding(
            get: { review.isViewed(route: agent.route, root: root, file: file) },
            set: { on in
                review.setViewed(on, route: agent.route, root: root, file: file)
                if on {
                    collapsed = true
                    editing = nil
                }
            }
        )
    }

    private var comments: [GitComment] {
        review.comments.filter { $0.root == root && $0.file.path == file.path && $0.file.section == file.section && $0.file.pullRequest == file.pullRequest }
    }

    private var header: some View {
        HStack(spacing: 8) {
            Button {
                collapsed.toggle()
                if collapsed { editing = nil }
            } label: {
                Image(systemName: collapsed ? "chevron.right" : "chevron.down")
                    .font(.footnote.weight(.semibold))
                    .frame(minWidth: 20, minHeight: 28)
            }
            .accessibilityLabel(collapsed ? "Expand diff" : "Collapse diff")
            HStack(spacing: 4) {
                if let old = file.oldPath { Text(verbatim: "\(old) →").layoutPriority(-1) }
                Text(verbatim: file.path).textSelection(.enabled)
            }
            .font(.footnote.weight(.semibold))
            .lineLimit(1)
            .truncationMode(.middle)
            Spacer(minLength: 0)
            if let additions = file.additions, let deletions = file.deletions {
                HStack(spacing: 4) {
                    Text(verbatim: "+\(additions)").foregroundStyle(.green)
                    Text(verbatim: "−\(deletions)").foregroundStyle(.red)
                }
                .font(.caption.monospacedDigit())
                .accessibilityElement(children: .ignore)
                .accessibilityLabel("\(additions) added, \(deletions) removed lines")
            }
            if file.hash != nil {
                Toggle(isOn: viewed) {
                    Label("Viewed", systemImage: viewed.wrappedValue ? "checkmark.circle.fill" : "circle")
                }
                .toggleStyle(.button)
                .labelStyle(.iconOnly)
                .buttonStyle(.glass)
            }
            Button("File feedback", systemImage: "text.bubble") {
                collapsed = false
                editing = review.add(root: root, file: file, selection: "")
            }
            .labelStyle(.iconOnly)
            .buttonStyle(.glass)
            .disabled(review.sending)
        }
        .controlSize(.small)
        .padding(.horizontal, 10)
        .padding(.vertical, 6)
        .glassEffect(.regular, in: .rect(cornerRadius: 16))
        .padding(.horizontal, 8)
        .padding(.bottom, 4)
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
    var cleanup: [GitWorktree]?
    var checkout: String?

    var body: some View {
        VStack(spacing: 8) {
            if let error = review.error {
                Text(error).font(.footnote).foregroundStyle(.red)
                    .padding(8).glassEffect(.regular, in: .rect(cornerRadius: 12))
            }
            if !agent.canSendText && !review.sending {
                Text(cleanup == nil ? "Feedback stays here until the agent can accept a prompt." : "Clean up waits until the agent can accept a prompt.")
                    .font(.footnote).foregroundStyle(.secondary)
                    .padding(8).glassEffect(.regular, in: .rect(cornerRadius: 12))
            }
            Button {
                Task {
                    let done = if let cleanup {
                        await review.cleanUp(cleanup, checkout: checkout ?? "", using: agent)
                    } else { await review.send(using: agent) }
                    if done { sent() }
                }
            } label: {
                if review.sending { ProgressView() }
                else if let cleanup { Label("Clean up (\(cleanup.count))", systemImage: "trash") }
                else { Label("Send feedback (\(review.comments.count))", systemImage: "text.bubble") }
            }
            .buttonStyle(.glassProminent)
            .onAccentFill()
            .controlSize(.large)
            .disabled(review.sending || !(cleanup.map { !$0.isEmpty } ?? review.ready) || !agent.canSendText)
        }
        .padding(12)
        .frame(maxWidth: .infinity)
        .dynamicTypeSize(...DynamicTypeSize.xxxLarge)
    }
}

private extension GitFile {
    static func sectionLabel(_ section: String) -> String {
        section == "pull_request" ? "Pull request" : section.capitalized
    }

    var statusMarker: String {
        switch status.first {
        case "A", "D", "R", "C", "T", "U", "?": String(status.prefix(1))
        default: "M"
        }
    }

    var statusColor: Color {
        switch status.first {
        case "A", "?": .green
        case "D": .red
        case "U": .orange
        default: .secondary
        }
    }

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
