import CollieCore
import SwiftUI

struct ApprovalCard: View {
    let model: ApprovalsModel
    let item: ApprovalItem
    var maxHeight: CGFloat = .infinity
    let sendAnswer: (String) async throws -> Void
    @State private var decisionsHeight: CGFloat = 0
    @State private var shownHeight: CGFloat = .infinity
    @State private var moreAbove = false
    @State private var moreBelow = false
    @State private var commandShown: CGFloat = 0
    @State private var commandFull: CGFloat = 0
    @State private var headerHeight: CGFloat = 0
    @FocusState private var field: String?
    @Environment(\.dynamicTypeSize) private var typeSize
    @ScaledMetric(relativeTo: .caption) private var commandLine: CGFloat = 15
    @ScaledMetric(relativeTo: .caption) private var fade: CGFloat = 16
    private let padding: CGFloat = 10
    private let spacing: CGFloat = 6

    private var approval: PendingApproval { item.approval }
    private var expanded: Bool { model.expanded.contains(item.id) }
    private var offersDecisions: Bool { !approval.answeredInTerminal && !approval.options.isEmpty }
    // Approve and Deny stay pinned under the scrolling prompt while the card's height leaves it two
    // command lines; below that (tight landscape, the keyboard up) they scroll with it, so all stays
    // reachable.
    private var pinsDecisions: Bool {
        offersDecisions && min(maxHeight - 2 * padding, shownHeight) - spacing - decisionsHeight >= 2 * commandLine
    }
    // Collapsed, the command keeps to the lines that leave the tool's name and Show full command in view at rest
    // (at accessibility sizes three lines pushed the button under the fade); when not even one would (landscape),
    // it keeps three and scrolls. Measured against the card's cap, not its height, which follows the command.
    private var commandLines: Int {
        guard maxHeight.isFinite else { return 3 }
        let room = maxHeight - 2 * padding - (pinsDecisions ? spacing + decisionsHeight : 0) - headerHeight - spacing - 2 * 8 - 4 - commandLine
        let lines = Int(room / commandLine)
        return lines < 1 ? 3 : min(3, lines)
    }

    var body: some View {
        UpToContent(maxHeight: max(0, maxHeight - 2 * padding)) {
            VStack(alignment: .leading, spacing: spacing) {
                ScrollViewReader { proxy in
                    UpToContent {
                        ScrollView {
                            VStack(alignment: .leading, spacing: spacing) {
                                details
                                if offersDecisions, !pinsDecisions { decisions }
                            }
                            .frame(maxWidth: .infinity, alignment: .leading)
                        }
                        .scrollBounceBehavior(.basedOnSize)
                        // A fade where the prompt goes on below: a hard cut at the edge read as a clipped snippet box.
                        .onScrollGeometryChange(for: Bool.self) { $0.visibleRect.maxY < $0.contentSize.height - 1 } action: { moreBelow = $1 }
                        .onScrollGeometryChange(for: Bool.self) { $0.visibleRect.minY > 1 } action: { moreAbove = $1 }
                        .mask {
                            VStack(spacing: 0) {
                                LinearGradient(colors: [.clear, .black], startPoint: .top, endPoint: .bottom).frame(height: moreAbove ? fade : 0)
                                Rectangle()
                                LinearGradient(colors: [.black, .clear], startPoint: .top, endPoint: .bottom).frame(height: moreBelow ? fade : 0)
                            }
                        }
                    }
                    .onChange(of: field) { _, field in
                        if let field { withAnimation { proxy.scrollTo(field, anchor: .bottom) } }
                    }
                }
                if pinsDecisions { decisions }
            }
        }
        .onGeometryChange(for: CGFloat.self) { $0.size.height } action: { shownHeight = $0 }
        .padding(.vertical, padding)
        .padding(.horizontal)
    }

    private var decisions: some View {
        TimelineView(.periodic(from: .now, by: 1)) { context in
            DecisionButtons(model: model, item: item, expired: approval.expiresAtMs <= context.date.unixMs)
        }
        .onGeometryChange(for: CGFloat.self) { $0.size.height } action: { decisionsHeight = $0 }
    }

    private var details: some View {
        VStack(alignment: .leading, spacing: spacing) {
            Button {
                withAnimation { model.toggleExpanded(item) }
            } label: {
                HStack(alignment: .center, spacing: 8) {
                    Image(systemName: "chevron.right")
                        .font(.caption2.weight(.semibold))
                        .foregroundStyle(.secondary)
                        .rotationEffect(.degrees(expanded ? 90 : 0))
                    // The session header above already names the agent and its machine.
                    Text(verbatim: approval.toolName ?? approval.agentLabel).font(.footnote.weight(.semibold)).lineLimit(1)
                    Spacer()
                    TimelineView(.periodic(from: .now, by: 1)) { context in
                        Label(Countdown.string(untilMs: approval.expiresAtMs, now: context.date), systemImage: "timer")
                            .font(.caption2.weight(.semibold).monospacedDigit())
                            .foregroundStyle(approval.expiresAtMs < context.date.unixMs + 60_000 ? AgentState.blocked.color : .secondary)
                            .padding(.horizontal, 6)
                            .padding(.vertical, 2)
                            .padding(2)
                            .glassEffect(.regular, in: .capsule)
                            .fixedSize()
                    }
                }
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .onGeometryChange(for: CGFloat.self) { $0.size.height } action: { headerHeight = $0 }
            .accessibilityHint(expanded ? "Shows less of the prompt" : "Shows the whole prompt")
            if approval.toolName != nil, let command = approval.toolSummary {
                // Lines break at spaces only: a word joiner after each hyphen keeps a flag such as `--nocapture` whole.
                let wrapped = command.replacingOccurrences(of: "-", with: "-\u{2060}")
                VStack(alignment: .leading, spacing: 4) {
                    Text(verbatim: wrapped)
                        .lineLimit(expanded ? nil : commandLines)
                        .fixedSize(horizontal: false, vertical: true)
                        .onGeometryChange(for: CGFloat.self) { $0.size.height } action: { commandShown = $0 }
                        .background(alignment: .top) {
                            Text(verbatim: wrapped)
                                .fixedSize(horizontal: false, vertical: true)
                                .hidden()
                                .onGeometryChange(for: CGFloat.self) { $0.size.height } action: { commandFull = $0 }
                        }
                        .accessibilityLabel(command)
                    if !expanded, commandFull > commandShown + 1 {
                        Button("Show full command") { withAnimation { model.toggleExpanded(item) } }
                            .font(.caption.weight(.semibold))
                            .buttonStyle(.borderless)
                    }
                }
                .font(.caption.monospaced())
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(8)
                .background(.fill.tertiary, in: RoundedRectangle(cornerRadius: 12))
            }
            if let reason = approval.reason {
                ApprovalReason(reason: reason)
            }
            if approval.options.isEmpty, !approval.choices.isEmpty {
                QuestionText(snippet: approval.snippet)
            } else if !approval.snippet.isEmpty, expanded || !typeSize.isAccessibilitySize {
                // Collapsed at accessibility sizes, the card's share of the screen left the output a sliver.
                Text(verbatim: approval.snippet)
                    .font(.caption2.monospaced())
                    .foregroundStyle(.secondary)
                    .lineLimit(expanded ? nil : 3)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(8)
                    .background(.fill.tertiary, in: RoundedRectangle(cornerRadius: 12))
                    .fixedSize(horizontal: false, vertical: true)
            }
            if approval.supportsNote, model.noting.contains(item.id) {
                NoteField(model: model, item: item, field: $field)
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
                    .foregroundStyle(.orange)
            }
            if approval.supportsNote, !model.noting.contains(item.id) {
                Button("Add note", systemImage: "text.bubble") { model.toggleNote(item) }
                    .font(.caption)
                    .disabled(model.steps[item.id] != nil)
            }
            if approval.takesFeedback {
                FeedbackField(model: model, item: item, field: $field, sendAnswer: sendAnswer)
            }
        }
    }
}

/// Its one subview's ideal height, at most `maxHeight` and what it is offered: unlike a frame's
/// `maxHeight`, it never grows past the content, and a scroll view in it is as tall as its content.
private struct UpToContent: Layout {
    var maxHeight: CGFloat = .infinity

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        let ideal = subviews[0].sizeThatFits(ProposedViewSize(width: proposal.width, height: nil))
        return CGSize(width: proposal.width ?? ideal.width, height: min(ideal.height, maxHeight, proposal.height ?? .infinity))
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        subviews[0].place(at: bounds.origin, proposal: ProposedViewSize(bounds.size))
    }
}

struct ApprovalReason: View {
    let reason: String
    @Environment(\.colorScheme) private var colorScheme

    var body: some View {
        Label {
            Text(verbatim: reason)
        } icon: {
            Image(systemName: "exclamationmark.shield.fill").foregroundStyle(.orange)
        }
        .font(.caption)
        .padding(8)
        .frame(maxWidth: .infinity, alignment: .leading)
        // Orange fill alone reads as muddy brown over dark glass; there an orange rim carries the warning.
        .background(.orange.opacity(colorScheme == .dark ? 0.1 : 0.14), in: RoundedRectangle(cornerRadius: 12))
        .overlay {
            if colorScheme == .dark { RoundedRectangle(cornerRadius: 12).strokeBorder(.orange.opacity(0.5), lineWidth: 1) }
        }
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
    let expired: Bool

    var body: some View {
        GlassEffectContainer(spacing: 8) {
            // Labels never shrink to fit: the decisions stack when one row would cut a label.
            ViewThatFits(in: .horizontal) {
                HStack(spacing: 8) { buttons }
                VStack(spacing: 8) { buttons }
            }
        }
        .disabled(expired || item.unreachable || model.steps[item.id] != nil)
    }

    private var buttons: some View {
        ForEach(item.approval.options, id: \.self) { decision in
            Button(role: decision == .deny ? .destructive : nil) {
                Task { await model.decide(item, decision) }
            } label: {
                // Each label is as wide as the widest, so the row is measured at the equal widths it lays out with.
                ZStack {
                    ForEach(item.approval.options, id: \.self) { Text($0.title).lineLimit(1).hidden() }
                    if model.steps[item.id] == .sending(decision) {
                        ProgressView()
                    } else {
                        Text(decision.title).lineLimit(1)
                    }
                }
                .frame(maxWidth: .infinity)
            }
            .modifier(DecisionStyle(decision: decision))
            .font(.footnote)
            .controlSize(.small)
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
        case .deny: content.buttonStyle(.glass).tint(.red)
        default: content.buttonStyle(.glass).tint(.primary)
        }
    }
}
