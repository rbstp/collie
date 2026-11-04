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
    @State private var commandBoxHeight: CGFloat = 0
    @State private var reasonHeight: CGFloat = 0
    @State private var outputHeight: CGFloat = 0
    @State private var addNoteHeight: CGFloat = 0
    @FocusState private var field: String?
    @ScaledMetric(relativeTo: .footnote) private var commandLine: CGFloat = 16
    // Capped: at the largest sizes the fade covered the whole line of the reason above the decisions.
    @ScaledMetric(relativeTo: .caption) private var scaledFade: CGFloat = 16
    private var fade: CGFloat { min(scaledFade, 24) }
    @ScaledMetric(relativeTo: .caption) private var reasonLine: CGFloat = 16
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
    // The prompt's room above pinned decisions, measured against the card's cap, not its height, which follows the prompt.
    private var room: CGFloat { maxHeight - 2 * padding - (pinsDecisions ? spacing + decisionsHeight : 0) }
    // Collapsed with the decisions pinned, the prompt shows at rest only what fits whole, in this order: the tool's name,
    // the command and Show full command, a line of the reason, the rest of the reason, Add note, the output. Expanded,
    // everything shows and scrolls.
    private var fitsAtRest: Bool { !expanded && pinsDecisions && maxHeight.isFinite }
    private var reasonMinimum: CGFloat { approval.reason == nil ? 0 : spacing + 2 * 8 + reasonLine }
    // When not even one command line fits with Show full command and a line of the reason, the command keeps one above
    // pinned decisions (the largest text sizes), so they stay in view, and three when everything scrolls (landscape).
    private var commandLines: Int {
        guard maxHeight.isFinite else { return 3 }
        let lines = Int((room - headerHeight - spacing - 2 * 8 - 4 - commandLine - reasonMinimum) / commandLine)
        return lines < 1 ? (pinsDecisions ? 1 : 3) : min(3, lines)
    }
    private var reasonLines: Int? {
        guard fitsAtRest else { return nil }
        return max(1, Int((room - headerHeight - spacing - commandBoxHeight - spacing - 2 * 8) / reasonLine))
    }
    private var shownAtRest: CGFloat { headerHeight + spacing + commandBoxHeight + (approval.reason == nil ? 0 : spacing + reasonHeight) }
    private var offersAddNote: Bool { approval.supportsNote && !model.noting.contains(item.id) }
    private var showsAddNote: Bool { !fitsAtRest || shownAtRest + spacing + addNoteHeight <= room }
    private var showsOutput: Bool {
        !fitsAtRest || shownAtRest + (offersAddNote && showsAddNote ? spacing + addNoteHeight : 0) + spacing + outputHeight <= room
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
                            .foregroundStyle(approval.expiresAtMs < context.date.unixMs + 60_000 ? AgentState.blocked.color : .metadata)
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
                // Lines break at spaces only: a word joiner after each hyphen keeps a flag such as `--nocapture` whole,
                // and a bare `--` stays with the argument after it.
                let wrapped = command.replacingOccurrences(of: " -- ", with: " --\u{00A0}").replacingOccurrences(of: "-", with: "-\u{2060}")
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
                        Button {
                            withAnimation { model.toggleExpanded(item) }
                        } label: {
                            // A second line at the largest text sizes cost the reason its place above the decisions.
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
                .onGeometryChange(for: CGFloat.self) { $0.size.height } action: { commandBoxHeight = $0 }
            }
            if let reason = approval.reason {
                ApprovalReason(reason: reason, lines: reasonLines)
                    .onGeometryChange(for: CGFloat.self) { $0.size.height } action: { reasonHeight = $0 }
            }
            if approval.options.isEmpty, !approval.choices.isEmpty {
                QuestionText(snippet: approval.snippet)
            } else if !approval.snippet.isEmpty, showsOutput {
                let lines = approval.snippet.split(separator: "\n", omittingEmptySubsequences: false)
                let more = expanded ? 0 : lines.count - 3
                VStack(alignment: .leading, spacing: 4) {
                    if expanded {
                        Text(verbatim: approval.snippet)
                    } else {
                        // One line each, so the count below is every line not shown.
                        VStack(alignment: .leading, spacing: 0) {
                            ForEach(lines.prefix(3).indices, id: \.self) { Text(verbatim: String(lines[$0])).lineLimit(1) }
                        }
                    }
                    if more > 0 {
                        Text(more == 1 ? "+1 line" : "+\(more) lines").font(.caption)
                    }
                }
                .font(.caption2.monospaced())
                // The metadata gray is 4.2:1 on a nested panel over the band's light tint; this is 5.3:1, and 5.5:1 in dark mode.
                .foregroundStyle(.primary.opacity(0.6))
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(.vertical, 8)
                .padding(.horizontal, 10)
                .background(.fill.tertiary, in: RoundedRectangle(cornerRadius: 12))
                .fixedSize(horizontal: false, vertical: true)
                .onGeometryChange(for: CGFloat.self) { $0.size.height } action: { outputHeight = $0 }
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
            if offersAddNote, showsAddNote {
                Button("Add note", systemImage: "text.bubble") { model.toggleNote(item) }
                    .font(.caption)
                    .disabled(model.steps[item.id] != nil)
                    .onGeometryChange(for: CGFloat.self) { $0.size.height } action: { addNoteHeight = $0 }
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
    var lines: Int?
    @Environment(\.colorScheme) private var colorScheme

    var body: some View {
        Label {
            Text(verbatim: reason).lineLimit(lines)
        } icon: {
            Image(systemName: "exclamationmark.shield.fill").foregroundStyle(.orange)
        }
        .font(.caption)
        .padding(8)
        .frame(maxWidth: .infinity, alignment: .leading)
        // Orange fill alone reads as muddy brown over dark glass; a faint fill inside an orange rim carries the warning,
        // fainter in dark mode, where 10% still read brown.
        .background(.orange.opacity(colorScheme == .dark ? 0.06 : 0.1), in: RoundedRectangle(cornerRadius: 12))
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
    let expired: Bool

    var body: some View {
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
        .disabled(expired || item.unreachable || model.steps[item.id] != nil)
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
        case .deny: content.buttonStyle(.glass).tint(AgentState.blocked.color)
        default: content.buttonStyle(.glass).tint(.primary)
        }
    }
}
