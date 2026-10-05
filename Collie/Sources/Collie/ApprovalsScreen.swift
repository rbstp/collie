import CollieCore
import SwiftUI

struct ApprovalsScreen: View {
    let model: ApprovalsModel

    var body: some View {
        NavigationStack {
            ScrollViewReader { proxy in
                List {
                    if let notice = model.notice {
                        Text(notice).font(.footnote)
                    }
                    if let loading = model.loading {
                        Section {
                            HStack(spacing: 12) {
                                ProgressView()
                                VStack(alignment: .leading, spacing: 2) {
                                    Text("Loading the approval")
                                    Text(verbatim: [loading.machine.label, loading.phase?.label].compactMap { $0 }.joined(separator: " · "))
                                        .font(.caption)
                                        .foregroundStyle(.secondary)
                                }
                            }
                        }
                    } else if model.items.isEmpty {
                        ContentUnavailableView(
                            "No pending approvals",
                            systemImage: "checkmark.shield",
                            description: Text("When an agent is blocked on a question, it shows up here.")
                        )
                    }
                    ForEach(model.items) { item in
                        Section {
                            ApprovalCard(model: model, item: item, showsMachine: true)
                        }
                        .id(item.id)
                        .listRowBackground(item.id == model.highlighted ? Color.accentColor.opacity(0.12) : nil)
                    }
                }
                .onChange(of: model.highlighted, initial: true) { _, id in
                    if let id { withAnimation { proxy.scrollTo(id, anchor: .top) } }
                }
                .onChange(of: model.items.map(\.id)) { _, _ in
                    if let id = model.highlighted, model.items.contains(where: { $0.id == id }) {
                        proxy.scrollTo(id, anchor: .top)
                    }
                }
            }
            .navigationTitle("Approvals")
            .navigationBarTitleDisplayMode(.inline)
            .refreshable { await model.refresh() }
            .toolbar {
                Button("Refresh", systemImage: "arrow.clockwise") {
                    Task { await model.refresh() }
                }
                .disabled(model.refreshing)
            }
            .task { await model.refresh() }
        }
    }
}

struct ApprovalCard: View {
    let model: ApprovalsModel
    let item: ApprovalItem
    var showsMachine = false

    private var approval: PendingApproval { item.approval }
    private var expanded: Bool { model.expanded.contains(item.id) }

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(alignment: .firstTextBaseline) {
                Image(systemName: "chevron.right")
                    .font(.caption.weight(.semibold))
                    .foregroundStyle(.secondary)
                    .rotationEffect(.degrees(expanded ? 90 : 0))
                VStack(alignment: .leading, spacing: 2) {
                    Text(verbatim: approval.agentLabel).font(.headline).lineLimit(1)
                    Text(verbatim: [approval.workspaceLabel, showsMachine ? item.machine.label : nil].compactMap { $0 }.joined(separator: " · "))
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                }
                Spacer()
                TimelineView(.periodic(from: .now, by: 1)) { context in
                    Text("\(Image(systemName: "timer")) \(Countdown.string(untilMs: approval.expiresAtMs, now: context.date))")
                        .font(.caption.monospacedDigit())
                        .foregroundStyle(.secondary)
                }
            }
            .contentShape(Rectangle())
            .onTapGesture { withAnimation { model.toggleExpanded(item) } }
            .accessibilityAddTraits(.isButton)
            .accessibilityHint(expanded ? "Shows less of the prompt" : "Shows the whole prompt")
            if let tool = approval.toolName {
                Text(verbatim: [tool, approval.toolSummary].compactMap { $0 }.joined(separator: ": "))
                    .font(.callout.monospaced())
                    .lineLimit(expanded ? nil : 3)
            }
            if approval.options.isEmpty, !approval.choices.isEmpty {
                QuestionText(snippet: approval.snippet)
            } else if !approval.snippet.isEmpty {
                Text(verbatim: approval.snippet)
                    .font(.caption.monospaced())
                    .foregroundStyle(.secondary)
                    .lineLimit(expanded ? nil : 8)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(8)
                    .background(.fill.tertiary, in: RoundedRectangle(cornerRadius: 8))
                    .fixedSize(horizontal: false, vertical: true)
            }
            if approval.supportsNote, model.noting.contains(item.id) {
                NoteField(model: model, item: item)
            }
            TimelineView(.periodic(from: .now, by: 1)) { context in
                let expired = approval.expiresAtMs <= UInt64(context.date.timeIntervalSince1970 * 1000)
                if approval.options.isEmpty {
                    ChoiceButtons(model: model, item: item, expired: expired)
                } else {
                    DecisionButtons(model: model, item: item, expired: expired)
                }
            }
            if item.unreachable {
                Label("\(item.machine.label) is unreachable", systemImage: "exclamationmark.triangle")
                    .font(.footnote)
                    .foregroundStyle(.orange)
            }
            if approval.supportsNote, !model.noting.contains(item.id) {
                Button("Add note", systemImage: "text.bubble") { model.toggleNote(item) }
                    .font(.footnote)
                    .disabled(model.steps[item.id] != nil)
            }
            if approval.takesFeedback {
                FeedbackField(model: model, item: item)
            }
        }
        .padding(.vertical, 4)
    }
}

extension ApprovalsModel {
    func draft(_ item: ApprovalItem) -> Binding<String> {
        // collied caps notes at 200 characters: a longer line cannot be read back once wrapped.
        Binding(get: { self.drafts[item.id, default: ""] }, set: { self.drafts[item.id] = String($0.prefix(200)) })
    }
}

/// Sent with Approve or Deny, typed on the Mac in place of the option's amend placeholder.
private struct NoteField: View {
    let model: ApprovalsModel
    let item: ApprovalItem

    var body: some View {
        HStack(spacing: 8) {
            TextField("Note for \(item.approval.agentLabel)", text: model.draft(item))
                .textFieldStyle(.roundedBorder)
                .submitLabel(.done)
            Button("Remove note", systemImage: "xmark.circle.fill") { model.toggleNote(item) }
                .labelStyle(.iconOnly)
                .buttonStyle(.plain)
                .foregroundStyle(.secondary)
        }
        .disabled(model.steps[item.id] != nil)
    }
}

/// The plan's "Tell Claude what to change", sent with `typeText`.
private struct FeedbackField: View {
    let model: ApprovalsModel
    let item: ApprovalItem

    var body: some View {
        HStack(spacing: 8) {
            TextField("Tell Claude what to change", text: model.draft(item))
                .textFieldStyle(.roundedBorder)
                .submitLabel(.send)
                .onSubmit { Task { await model.sendFeedback(item) } }
            Button {
                Task { await model.sendFeedback(item) }
            } label: {
                if model.steps[item.id] == .typing {
                    ProgressView().frame(width: 28, height: 28)
                } else {
                    Image(systemName: "arrow.up.circle.fill").font(.system(size: 28))
                }
            }
            .buttonStyle(.plain)
            .disabled(model.drafts[item.id, default: ""].trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
            .accessibilityLabel("Send feedback")
        }
        .disabled(item.unreachable || model.steps[item.id] != nil)
    }
}

private struct DecisionButtons: View {
    let model: ApprovalsModel
    let item: ApprovalItem
    let expired: Bool

    var body: some View {
        HStack(spacing: 8) {
            ForEach(item.approval.options, id: \.self) { decision in
                Button(role: decision == .deny ? .destructive : nil) {
                    Task { await model.decide(item, decision) }
                } label: {
                    Group {
                        if model.steps[item.id] == .sending(decision) {
                            ProgressView()
                        } else {
                            Text(decision.title).lineLimit(1).minimumScaleFactor(0.6)
                        }
                    }
                    .frame(maxWidth: .infinity)
                }
                .modifier(DecisionStyle(prominent: decision == .approve))
                .disabled(model.note(for: item) != nil && !item.approval.takesNote(with: decision))
            }
        }
        .disabled(expired || item.unreachable || model.steps[item.id] != nil)
    }
}

/// The menu on the Mac, for prompts collied offers no Approve/Deny for; each pick goes through `approval.decide`.
/// A question's header line (Claude Code draws it after a ☐) above the question itself.
private struct QuestionText: View {
    let snippet: String

    var body: some View {
        let lines = snippet.split(separator: "\n").map(String.init)
        let header = lines.first.flatMap { $0.hasPrefix("☐") ? String($0.dropFirst()).trimmingCharacters(in: .whitespaces) : nil }
        let question = (header == nil ? lines : Array(lines.dropFirst())).joined(separator: "\n")
        VStack(alignment: .leading, spacing: 2) {
            if let header, !header.isEmpty {
                Text(verbatim: header).font(.caption.weight(.medium)).foregroundStyle(.secondary)
            }
            if !question.isEmpty {
                Text(verbatim: question).font(.subheadline.weight(.semibold))
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .fixedSize(horizontal: false, vertical: true)
    }
}

private struct ChoiceButtons: View {
    let model: ApprovalsModel
    let item: ApprovalItem
    let expired: Bool

    private var disabled: Bool { expired || item.unreachable || model.steps[item.id] != nil }

    var body: some View {
        VStack(spacing: 0) {
            ForEach(item.approval.choices, id: \.index) { choice in
                let decision = ApprovalDecision.choose(choice: choice.index)
                if choice.index > 0 {
                    Divider().padding(.leading, 38)
                }
                Button {
                    Task { await model.decide(item, decision) }
                } label: {
                    HStack(alignment: .firstTextBaseline, spacing: 10) {
                        Text(verbatim: "\(Int(choice.index) + 1)")
                            .font(.subheadline.monospacedDigit().weight(.semibold))
                            .foregroundStyle(choice.current ? Color.accentColor : .secondary)
                            .frame(width: 18, alignment: .trailing)
                        VStack(alignment: .leading, spacing: 1) {
                            Text(verbatim: choice.label).font(.subheadline)
                            if let detail = choice.detail {
                                Text(verbatim: detail).font(.caption).foregroundStyle(.secondary)
                            }
                        }
                        .multilineTextAlignment(.leading)
                        Spacer(minLength: 0)
                        if model.steps[item.id] == .sending(decision) {
                            ProgressView().controlSize(.small)
                        }
                    }
                    .padding(.vertical, 7)
                    .padding(.horizontal, 10)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .background(choice.current ? Color.accentColor.opacity(0.12) : .clear)
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .accessibilityValue(choice.current ? "Under the cursor on the machine" : "")
            }
        }
        .background(.fill.tertiary)
        .clipShape(RoundedRectangle(cornerRadius: 10))
        .fixedSize(horizontal: false, vertical: true)
        .opacity(disabled ? 0.5 : 1)
        .disabled(disabled)
    }
}

private struct DecisionStyle: ViewModifier {
    let prominent: Bool

    func body(content: Content) -> some View {
        if prominent {
            content.buttonStyle(.borderedProminent)
        } else {
            content.buttonStyle(.bordered)
        }
    }
}
