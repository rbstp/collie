import CollieCore
import SwiftUI

struct ApprovalCard: View {
    let model: ApprovalsModel
    let item: ApprovalItem
    var maxHeight: CGFloat = .infinity
    let sendAnswer: (String) async throws -> Void
    @FocusState private var field: String?
    @Environment(\.verticalSizeClass) private var verticalSizeClass
    private let padding: CGFloat = 10

    private var approval: PendingApproval { item.approval }
    private var offersDecisions: Bool { !approval.answeredInTerminal && !approval.options.isEmpty }

    var body: some View {
        // The whole card when it fits its share of the screen. Otherwise (landscape, larger text) a band with the tool, a
        // line of the command and Review, and the decisions when they fit too: Review opens the whole prompt in a sheet.
        Capped(maxHeight: max(0, maxHeight - 2 * padding)) {
            ViewThatFits(in: .vertical) {
                VStack(alignment: .leading, spacing: 6) {
                    ApprovalDetails(model: model, item: item, whole: false, field: $field, sendAnswer: sendAnswer)
                    if offersDecisions { DecisionButtons(model: model, item: item) }
                }
                if offersDecisions { band(decisions: true) }
                band(decisions: false)
            }
        }
        .padding(.vertical, padding)
        .padding(.horizontal)
    }

    private func band(decisions: Bool) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            let row = HStack(spacing: 8) {
                toolName
                command
                reviewAndCountdown
            }
            // In landscape one row is all the band has room for, the command cut to fit; in portrait the command takes a
            // row of its own when it does not fit whole beside the tool's name.
            if verticalSizeClass == .compact {
                row
            } else {
                ViewThatFits(in: .horizontal) {
                    row
                    VStack(alignment: .leading, spacing: 6) {
                        HStack(spacing: 8) {
                            toolName
                            Spacer(minLength: 0)
                            reviewAndCountdown
                        }
                        command
                    }
                }
            }
            if decisions { DecisionButtons(model: model, item: item) }
        }
    }

    private var toolName: some View {
        Text(verbatim: approval.toolName ?? approval.agentLabel).font(.footnote.weight(.semibold)).lineLimit(1).fixedSize()
    }

    @ViewBuilder private var command: some View {
        if approval.toolName != nil, let command = approval.toolSummary {
            Text(verbatim: command)
                .font(.footnote.monospaced())
                .lineLimit(1)
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(.vertical, 4)
                .padding(.horizontal, 10)
                .background(.fill.tertiary, in: RoundedRectangle(cornerRadius: 12))
        } else {
            Spacer(minLength: 0)
        }
    }

    private var reviewAndCountdown: some View {
        HStack(spacing: 8) {
            ApprovalCountdown(approval: approval)
            Button {
                model.toggleExpanded(item)
            } label: {
                // The reason waits in the sheet; its shield says there is one.
                Label {
                    Text("Review")
                } icon: {
                    if approval.reason != nil { Image(systemName: "exclamationmark.shield.fill").foregroundStyle(.warning) }
                }
                .labelStyle(.titleAndIcon)
                .font(.footnote.weight(.semibold))
            }
            .buttonStyle(.glass)
            .tint(.primary)
            .accessibilityHint(approval.reason == nil ? "Shows the whole prompt" : "Shows the whole prompt and why the agent asks")
        }
        .fixedSize()
    }
}

/// Proposes at most `maxHeight` to its one subview, so a `ViewThatFits` in it takes the first view that fits the card's share.
private struct Capped: Layout {
    let maxHeight: CGFloat

    private func capped(_ proposal: ProposedViewSize, width: CGFloat?) -> ProposedViewSize {
        ProposedViewSize(width: width, height: min(maxHeight, proposal.height ?? .infinity))
    }

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        subviews[0].sizeThatFits(capped(proposal, width: proposal.width))
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        subviews[0].place(at: bounds.origin, proposal: capped(proposal, width: bounds.width))
    }
}

/// The whole prompt with its decisions in reach, opened from the card.
struct ApprovalReview: View {
    let model: ApprovalsModel
    let item: ApprovalItem
    let task: String
    let sendAnswer: (String) async throws -> Void
    @FocusState private var field: String?
    @Environment(\.dismiss) private var dismiss
    @Environment(\.dynamicTypeSize) private var typeSize

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(alignment: .leading, spacing: 10) {
                    // Full height, the sheet hides the session header: the task and its machine say who gets the answer.
                    VStack(alignment: .leading, spacing: 2) {
                        Text(verbatim: task).font(.subheadline.weight(.semibold))
                        Text(verbatim: item.machine.label).font(.caption).foregroundStyle(.metadata)
                    }
                    .accessibilityElement(children: .combine)
                    ApprovalDetails(model: model, item: item, whole: true, field: $field, sendAnswer: sendAnswer)
                }
                .padding(.horizontal)
                .padding(.bottom)
            }
            // The soft edge left the prompt reading through the decisions' glass.
            .scrollEdgeEffectStyle(.hard, for: .bottom)
            .safeAreaBar(edge: .bottom) {
                let decides = !item.approval.answeredInTerminal && !item.approval.options.isEmpty
                // The session's notice line sits behind the sheet: a failed or unconfirmed decision is reported here.
                if decides || model.notice != nil {
                    VStack(spacing: 8) {
                        if let notice = model.notice {
                            Text(notice).font(.footnote).frame(maxWidth: .infinity, alignment: .leading)
                        }
                        if decides { DecisionButtons(model: model, item: item) }
                    }
                    .padding()
                }
            }
            .navigationTitle(item.approval.toolName ?? item.approval.agentLabel)
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button(role: .close) { dismiss() }.tint(.primary)
                }
                ToolbarItem(placement: .primaryAction) {
                    ApprovalCountdown(approval: item.approval)
                }
                .sharedBackgroundVisibility(.hidden)
            }
        }
        // At half height, stacked decisions left larger text no room for the reason.
        .presentationDetents(typeSize.isAccessibilitySize ? [.large] : [.medium, .large])
    }
}

/// The prompt: in the card, the command cut to three lines and the output a link to the review sheet; in the sheet, whole.
private struct ApprovalDetails: View {
    let model: ApprovalsModel
    let item: ApprovalItem
    let whole: Bool
    let field: FocusState<String?>.Binding
    let sendAnswer: (String) async throws -> Void
    @State private var commandShown: CGFloat = 0
    @State private var commandFull: CGFloat = 0

    private var approval: PendingApproval { item.approval }

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            if !whole {
                Button {
                    model.toggleExpanded(item)
                } label: {
                    HStack(alignment: .center, spacing: 8) {
                        Image(systemName: "chevron.right")
                            .font(.caption2.weight(.semibold))
                            .foregroundStyle(.secondary)
                        // The session header above already names the agent and its machine.
                        Text(verbatim: approval.toolName ?? approval.agentLabel).font(.footnote.weight(.semibold)).lineLimit(1)
                        Spacer()
                        ApprovalCountdown(approval: approval)
                    }
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .accessibilityHint("Shows the whole prompt")
            }
            if approval.toolName != nil, let command = approval.toolSummary {
                // Lines break at spaces only: a word joiner after each hyphen keeps a flag such as `--nocapture` whole,
                // and a bare `--` stays with the argument after it.
                let wrapped = command.replacingOccurrences(of: " -- ", with: " --\u{00A0}").replacingOccurrences(of: "-", with: "-\u{2060}")
                VStack(alignment: .leading, spacing: 4) {
                    Text(verbatim: wrapped)
                        .lineLimit(whole ? nil : 3)
                        .fixedSize(horizontal: false, vertical: true)
                        .onGeometryChange(for: CGFloat.self) { $0.size.height } action: { commandShown = $0 }
                        .background(alignment: .top) {
                            Text(verbatim: wrapped)
                                .fixedSize(horizontal: false, vertical: true)
                                .hidden()
                                .onGeometryChange(for: CGFloat.self) { $0.size.height } action: { commandFull = $0 }
                        }
                        .accessibilityLabel(command)
                    if !whole, commandFull > commandShown + 1 {
                        Button {
                            model.toggleExpanded(item)
                        } label: {
                            ViewThatFits(in: .horizontal) {
                                Text("Show full command")
                                Text("Full command")
                            }
                        }
                        .font(.caption)
                        .foregroundStyle(.primary.opacity(0.6))
                        .buttonStyle(.plain)
                        .accessibilityLabel("Show full command")
                    }
                }
                .font(.footnote.monospaced())
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(.vertical, 8)
                .padding(.horizontal, 10)
                .background(.fill.tertiary, in: RoundedRectangle(cornerRadius: 12))
            }
            if let reason = approval.reason {
                ApprovalReason(reason: reason)
            }
            if approval.options.isEmpty, !approval.choices.isEmpty {
                QuestionText(snippet: approval.snippet)
            } else if !approval.snippet.isEmpty, !whole {
                let count = approval.snippet.split(separator: "\n", omittingEmptySubsequences: false).count
                Button {
                    model.toggleExpanded(item)
                } label: {
                    HStack(spacing: 4) {
                        Text(count == 1 ? "Output, 1 line" : "Output, \(count) lines")
                        Image(systemName: "chevron.right").font(.caption2.weight(.semibold))
                    }
                    .contentShape(Rectangle())
                }
                .font(.caption)
                .foregroundStyle(.primary.opacity(0.6))
                .buttonStyle(.plain)
                .accessibilityHint("Shows the whole prompt")
            } else if !approval.snippet.isEmpty {
                Text(verbatim: approval.snippet)
                    .font(.caption2.monospaced())
                    // The metadata gray is 4.2:1 on a nested panel over the band's light tint; this is 5.3:1, and 5.5:1 in dark mode.
                    .foregroundStyle(.primary.opacity(0.6))
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(.vertical, 8)
                    .padding(.horizontal, 10)
                    .background(.fill.tertiary, in: RoundedRectangle(cornerRadius: 12))
                    .fixedSize(horizontal: false, vertical: true)
            }
            if approval.supportsNote, model.noting.contains(item.id) {
                NoteField(model: model, item: item, field: field)
            }
            if approval.answeredInTerminal {
                Label("Use the terminal buttons or type an answer", systemImage: "terminal")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            } else if approval.options.isEmpty {
                TimelineView(.periodic(from: .now, by: 1)) { context in
                    ChoiceButtons(model: model, item: item, expired: approval.expiresAtMs <= context.date.unixMs)
                }
            }
            if item.unreachable {
                Label("\(item.machine.label) is unreachable", systemImage: "exclamationmark.triangle")
                    .font(.caption)
                    .foregroundStyle(.warning)
            }
            if approval.supportsNote, !model.noting.contains(item.id) {
                Button("Add note", systemImage: "text.bubble") { model.toggleNote(item) }
                    .font(.caption)
                    .disabled(model.steps[item.id] != nil)
            }
            if approval.takesFeedback {
                FeedbackField(model: model, item: item, field: field, sendAnswer: sendAnswer)
            }
        }
    }
}

private struct ApprovalCountdown: View {
    let approval: PendingApproval

    var body: some View {
        TimelineView(.periodic(from: .now, by: 1)) { context in
            Label(Countdown.string(untilMs: approval.expiresAtMs, now: context.date), systemImage: "timer")
                .labelStyle(.titleAndIcon)
                .font(.caption2.weight(.semibold).monospacedDigit())
                .foregroundStyle(approval.expiresAtMs < context.date.unixMs + 60_000 ? AgentState.blocked.color : .metadata)
                .padding(.horizontal, 6)
                .padding(.vertical, 2)
                .padding(2)
                .glassEffect(.regular, in: .capsule)
                .fixedSize()
        }
    }
}

struct ApprovalReason: View {
    let reason: String
    @Environment(\.colorScheme) private var colorScheme

    var body: some View {
        Label {
            Text(verbatim: reason)
        } icon: {
            Image(systemName: "exclamationmark.shield.fill").foregroundStyle(.warning)
        }
        .font(.caption)
        .padding(8)
        .frame(maxWidth: .infinity, alignment: .leading)
        // An orange rim carries the warning. In dark mode the fill is the command panel's: even a faint orange fill read
        // warm brown over dark glass.
        .background(colorScheme == .dark ? AnyShapeStyle(.fill.tertiary) : AnyShapeStyle(.orange.opacity(0.1)), in: RoundedRectangle(cornerRadius: 12))
        .overlay { RoundedRectangle(cornerRadius: 12).strokeBorder(.orange.opacity(0.4), lineWidth: 1) }
        .fixedSize(horizontal: false, vertical: true)
        .accessibilityElement(children: .combine)
        .accessibilityLabel("Reason: \(reason)")
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
    let field: FocusState<String?>.Binding

    var body: some View {
        HStack(spacing: 8) {
            TextField("Note for \(item.approval.agentLabel)", text: model.draft(item))
                .textFieldStyle(.roundedBorder)
                .font(.footnote)
                .submitLabel(.done)
                .focused(field, equals: "note")
            Button("Remove note", systemImage: "xmark.circle.fill") { model.toggleNote(item) }
                .labelStyle(.iconOnly)
                .buttonStyle(.plain)
                .foregroundStyle(.secondary)
        }
        .id("note")
        .disabled(model.steps[item.id] != nil)
    }
}

private struct FeedbackField: View {
    let model: ApprovalsModel
    let item: ApprovalItem
    let field: FocusState<String?>.Binding
    let sendAnswer: (String) async throws -> Void

    var body: some View {
        HStack(spacing: 8) {
            TextField("Answer this prompt", text: model.draft(item))
                .textFieldStyle(.roundedBorder)
                .font(.footnote)
                .submitLabel(.send)
                .focused(field, equals: "answer")
                .onSubmit { Task { await model.sendFeedback(item, send: sendAnswer) } }
            Button {
                Task { await model.sendFeedback(item, send: sendAnswer) }
            } label: {
                if model.steps[item.id] == .typing {
                    ProgressView().frame(width: 20, height: 20)
                } else {
                    Image(systemName: "arrow.up").font(.system(size: 17)).frame(width: 20, height: 20)
                }
            }
            .buttonStyle(.glassProminent)
            .onAccentFill()
            .buttonBorderShape(.circle)
            .controlSize(.small)
            .disabled(model.drafts[item.id, default: ""].trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
            .accessibilityLabel("Send answer")
        }
        .id("answer")
        .disabled(item.unreachable || model.steps[item.id] != nil)
    }
}

private struct DecisionButtons: View {
    let model: ApprovalsModel
    let item: ApprovalItem

    var body: some View {
        TimelineView(.periodic(from: .now, by: 1)) { context in
            GlassEffectContainer(spacing: 8) {
                // Labels never shrink to fit: the decisions take two rows, then stack, when a row would cut a label.
                ViewThatFits(in: .horizontal) {
                    HStack(spacing: 8) { buttons(options) }
                    VStack(spacing: 8) {
                        buttons(Array(options.prefix(1)))
                        HStack(spacing: 8) { buttons(Array(options.dropFirst())) }
                    }
                    VStack(spacing: 8) { buttons(options) }
                }
            }
            .disabled(item.approval.expiresAtMs <= context.date.unixMs || item.unreachable || model.steps[item.id] != nil)
        }
    }

    private var options: [ApprovalDecision] { item.approval.options }

    private func weight(_ decision: ApprovalDecision) -> Font.Weight { decision == .approve ? .semibold : .regular }

    private func buttons(_ decisions: [ApprovalDecision]) -> some View {
        ForEach(decisions, id: \.self) { decision in
            Button(role: decision == .deny ? .destructive : nil) {
                Task { await model.decide(item, decision) }
            } label: {
                // Each label is as wide as the widest, so the row is measured at the equal widths it lays out with.
                ZStack {
                    ForEach(item.approval.options, id: \.self) { Text($0.title).fontWeight(weight($0)).lineLimit(1).hidden() }
                    if model.steps[item.id] == .sending(decision) {
                        ProgressView()
                    } else {
                        Text(decision.title).fontWeight(weight(decision)).lineLimit(1)
                    }
                }
                .padding(.horizontal, 8)
                .frame(maxWidth: .infinity, minHeight: 22)
            }
            .modifier(DecisionStyle(decision: decision))
            .font(.footnote)
            .controlSize(.regular)
            .disabled(model.note(for: item) != nil && !item.approval.takesNote(with: decision))
        }
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
                Text(verbatim: header).font(.caption2.weight(.medium)).foregroundStyle(.secondary)
            }
            if !question.isEmpty {
                Text(verbatim: question).font(.footnote.weight(.semibold))
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
                    Divider().padding(.leading, 36)
                }
                Button {
                    Task { await model.decide(item, decision) }
                } label: {
                    HStack(alignment: .firstTextBaseline, spacing: 10) {
                        Text(verbatim: "\(Int(choice.index) + 1)")
                            .font(.footnote.monospacedDigit().weight(.semibold))
                            .foregroundStyle(choice.current ? Color.accentColor : .secondary)
                            .frame(width: 18, alignment: .trailing)
                        VStack(alignment: .leading, spacing: 1) {
                            Text(verbatim: choice.label).font(.footnote)
                            if let detail = choice.detail {
                                Text(verbatim: detail).font(.caption2).foregroundStyle(.secondary)
                            }
                        }
                        .multilineTextAlignment(.leading)
                        Spacer(minLength: 0)
                        if model.steps[item.id] == .sending(decision) {
                            ProgressView().controlSize(.small)
                        }
                    }
                    .padding(.vertical, 5)
                    .padding(.horizontal, 8)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .background(choice.current ? Color.accentColor.opacity(0.12) : .clear)
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .accessibilityValue(choice.current ? "Under the cursor on the machine" : "")
            }
        }
        .background(.fill.tertiary)
        .clipShape(RoundedRectangle(cornerRadius: 12))
        .fixedSize(horizontal: false, vertical: true)
        .opacity(disabled ? 0.5 : 1)
        .disabled(disabled)
    }
}

private struct DecisionStyle: ViewModifier {
    let decision: ApprovalDecision

    func body(content: Content) -> some View {
        switch decision {
        // System green under white text reads poorly; this deeper green keeps the label at 4.5:1 or more.
        case .approve: content.buttonStyle(.glassProminent).tint(Color(red: 0.11, green: 0.5, blue: 0.22))
        // Glass over the terminal came out lighter than over Chat, and the red label read 3.1:1 there; the dimming color
        // behind the glass keeps it at 4.5:1 or more over either.
        case .deny: content.buttonStyle(.glass).tint(AgentState.blocked.color).background(SessionBarDim(), in: .capsule)
        default: content.buttonStyle(.glass).tint(.primary).background(SessionBarDim(), in: .capsule)
        }
    }
}
