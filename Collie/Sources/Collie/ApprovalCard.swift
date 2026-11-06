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
                    // The session header above already names the agent and its machine.
                    HStack(spacing: 8) {
                        toolName
                        Spacer(minLength: 0)
                        reviewAndCountdown
                    }
                    ApprovalDetails(model: model, item: item, whole: false, field: $field, sendAnswer: sendAnswer)
                    if offersDecisions { DecisionButtons(model: model, item: item) }
                }
                .onAppear { model.setReasonLeftOut(item, false) }
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
        .onAppear { model.setReasonLeftOut(item, approval.reason != nil) }
    }

    private var toolName: some View {
        Text(verbatim: approval.toolName ?? approval.agentLabel).font(.footnote.weight(.semibold)).lineLimit(1).fixedSize()
    }

    @ViewBuilder private var command: some View {
        // The band leaves out the reason, and may cut the command.
        if approval.toolName != nil, let command = approval.toolSummary {
            Text(verbatim: command)
                .lineLimit(1)
                .modifier(CutReport(text: command) { model.setCommandCut(item, $0) })
                .font(.footnote.monospaced())
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(.vertical, 4)
                .padding(.horizontal, 10)
                .background(.fill.tertiary, in: RoundedRectangle(cornerRadius: 12))
        } else {
            Spacer(minLength: 0)
        }
    }

    private var reviewAndCountdown: some View {
        // Left out of the row until it shows: hidden inside its own view, it still took the row's spacing.
        TimelineView(.periodic(from: .now, by: 1)) { context in
            HStack(spacing: 8) {
                if Countdown.shows(untilMs: approval.expiresAtMs, now: context.date) {
                    ApprovalCountdown(approval: approval)
                }
                review
            }
            .fixedSize()
        }
    }

    private var review: some View {
        Button {
            model.toggleExpanded(item)
        } label: {
            // The band leaves the reason to the sheet; the shield says there is one.
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
        .background(SessionBarDim(), in: .capsule)
        // A 44-point target around the 30-point glass, so the row keeps its height. Where it reaches the decisions in
        // the band, they sit in front and take the tap.
        .contentShape(.interaction, Rectangle().inset(by: -7.5))
        .accessibilityHint(approval.reason == nil ? "Shows the whole prompt" : "Shows the whole prompt and why the agent asks")
    }
}

/// Proposes at most `maxHeight` to its one subview, so a `ViewThatFits` in it takes the first view that fits the card's share.
private struct Capped: Layout {
    let maxHeight: CGFloat

    private func capped(_ proposal: ProposedViewSize, width: CGFloat?) -> ProposedViewSize {
        // The card's padding comes back from the parent as a hair under the height the card asked for (217.66666666666663
        // for 217.66666666666666), and the whole card no longer fit it.
        ProposedViewSize(width: width, height: min(maxHeight, (proposal.height ?? .infinity) + 0.001))
    }

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        subviews[0].sizeThatFits(capped(proposal, width: proposal.width))
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        subviews[0].place(at: bounds.origin, proposal: capped(proposal, width: bounds.width))
    }
}

/// Calls `action` with whether the text it modifies, held to its line limit, leaves out lines of `text`.
private struct CutReport: ViewModifier {
    let text: String
    let action: (Bool) -> Void
    @State private var shown: CGFloat = 0
    @State private var whole: CGFloat = 0

    func body(content: Content) -> some View {
        content
            .onGeometryChange(for: CGFloat.self) { $0.size.height } action: { shown = $0 }
            .background {
                Text(verbatim: text)
                    .lineLimit(nil)
                    .fixedSize(horizontal: false, vertical: true)
                    .hidden()
                    .onGeometryChange(for: CGFloat.self) { $0.size.height } action: { whole = $0 }
            }
            .onChange(of: whole > shown + 1, initial: true) { action($1) }
    }
}

/// The whole prompt with its decisions in reach, opened from the card.
struct ApprovalReview: View {
    let model: ApprovalsModel
    let item: ApprovalItem
    let task: String
    let showsMachine: Bool
    let sendAnswer: (String) async throws -> Void
    @FocusState private var field: String?
    @Environment(\.dismiss) private var dismiss
    @Environment(\.dynamicTypeSize) private var typeSize
    @Environment(\.verticalSizeClass) private var verticalSizeClass
    @State private var more = false
    @State private var detent = PresentationDetent.medium
    @State private var fit: CGFloat?
    @State private var medium: CGFloat?
    @State private var bar: CGFloat = 0
    @State private var page: CGFloat = 0
    @State private var prompt: CGFloat = 0
    @State private var footer: CGFloat = 0

    /// Landscape and accessibility sizes always show the whole sheet, and so does the keyboard, which lifts the sheet over
    /// the card while the detent still reads half.
    private var half: Bool {
        detent != .large && verticalSizeClass != .compact && !typeSize.isAccessibilitySize && field == nil
    }

    /// As tall as the content at half height, and never taller than the medium detent: at medium a short output left
    /// about 150 points of the sheet empty. A custom detent reading a measured height from the environment never followed it.
    private var halfDetent: PresentationDetent {
        if let fit, let medium, fit < medium - 1 { .height(fit) } else { .medium }
    }

    /// The navigation bar, the prompt and the decisions at half height; the medium detent's height while the sheet is on it.
    private func measure() {
        guard half else { return }
        if detent == .medium, bar > 0, page > 0 { medium = bar + page }
        fit = bar + prompt + footer
    }

    var body: some View {
        // At half height the session header above, dimmed, still shows the task, and the card the command and the reason
        // unless it cut them: the sheet leaves to them what they show whole.
        let showsCommand = !half || model.commandCut.contains(item.id)
        let showsReason = !half || model.reasonLeftOut.contains(item.id)
        NavigationStack {
            // The decisions sit under the prompt, not over it: a bar over the scroll view, even over a hard edge, let the
            // prompt read through.
            VStack(spacing: 0) {
                ScrollView {
                    VStack(alignment: .leading, spacing: 10) {
                        // Full height, the sheet hides the session header: the task, and its machine when more than one is
                        // paired, say who gets the answer.
                        if !half {
                            VStack(alignment: .leading, spacing: 2) {
                                Text(verbatim: task).font(.subheadline)
                                if showsMachine {
                                    // The metadata gray read 4.2:1 on the half-height sheet's light glass.
                                    Text(verbatim: item.machine.label).font(.caption).foregroundStyle(.primary.opacity(0.6))
                                }
                            }
                            .accessibilityElement(children: .combine)
                        }
                        ApprovalDetails(model: model, item: item, whole: true, showsCommand: showsCommand, showsReason: showsReason, field: $field, sendAnswer: sendAnswer)
                    }
                    .padding(.horizontal)
                    .padding(.bottom)
                }
                .onScrollGeometryChange(for: Bool.self) { $0.visibleRect.maxY < $0.contentSize.height - 1 } action: { more = $1 }
                .onScrollGeometryChange(for: CGFloat.self) { $0.contentSize.height } action: {
                    prompt = $1
                    measure()
                }
                // A row cut by the sheet's edge read as the end of the prompt: a fade on the scroll view's own edge says more follows.
                .mask {
                    VStack(spacing: 0) {
                        Color.black
                        LinearGradient(colors: [.black, .clear], startPoint: .top, endPoint: .bottom).frame(height: more ? 24 : 0)
                    }
                }
                let decides = !item.approval.answeredInTerminal && !item.approval.options.isEmpty
                // The session's notice line sits behind the sheet: a failed or unconfirmed decision is reported here.
                let notice = model.noticeItem?.id == item.id ? model.notice : nil
                if decides || notice != nil {
                    VStack(spacing: 8) {
                        if let notice {
                            ApprovalNotice(notice: notice)
                        }
                        if decides {
                            // A note goes with the decision, so it sits with the decisions. In landscape Add note leads
                            // their row: a row of its own left the output two lines.
                            let note = item.approval.supportsNote
                            if note, verticalSizeClass == .compact, !typeSize.isAccessibilitySize, !model.noting.contains(item.id) {
                                HStack(spacing: 8) {
                                    NoteField(model: model, item: item, field: $field).fixedSize()
                                    DecisionButtons(model: model, item: item)
                                }
                            } else {
                                if note { NoteField(model: model, item: item, field: $field) }
                                DecisionButtons(model: model, item: item)
                            }
                        }
                    }
                    .padding()
                    .onGeometryChange(for: CGFloat.self) { $0.size.height } action: {
                        footer = $0
                        measure()
                    }
                }
            }
            .onGeometryChange(for: CGFloat.self) { $0.size.height } action: {
                page = $0
                measure()
            }
            .background {
                Color.clear.ignoresSafeArea().onGeometryChange(for: CGFloat.self) { $0.safeAreaInsets.top } action: {
                    bar = $0
                    measure()
                }
            }
            .navigationTitle(item.approval.toolName ?? item.approval.agentLabel)
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button(role: .close) { dismiss() }.tint(.primary)
                }
                // At half height the card above shows the countdown.
                if !half {
                    ToolbarItem(placement: .primaryAction) {
                        ApprovalCountdown(approval: item.approval).fixedSize()
                    }
                    .sharedBackgroundVisibility(.hidden)
                }
            }
        }
        // At half height, stacked decisions left larger text no room for the reason.
        .presentationDetents(typeSize.isAccessibilitySize ? [.large] : [halfDetent, .large], selection: $detent)
        .onChange(of: halfDetent) { old, new in
            if detent == old { detent = new }
        }
    }
}

/// The prompt: in the card, the command cut to three lines and the output left to the review sheet; in the sheet, whole.
private struct ApprovalDetails: View {
    let model: ApprovalsModel
    let item: ApprovalItem
    let whole: Bool
    var showsCommand = true
    var showsReason = true
    let field: FocusState<String?>.Binding
    let sendAnswer: (String) async throws -> Void

    private var approval: PendingApproval { item.approval }

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            if showsCommand, approval.toolName != nil, let command = approval.toolSummary {
                // Lines break at spaces only: a word joiner after each hyphen keeps a flag such as `--nocapture` whole,
                // and a bare `--` stays with the argument after it.
                let wrapped = command.replacingOccurrences(of: " -- ", with: " --\u{00A0}").replacingOccurrences(of: "-", with: "-\u{2060}")
                Text(verbatim: wrapped)
                    .lineLimit(whole ? nil : 3)
                    .fixedSize(horizontal: false, vertical: true)
                    .modifier(CutReport(text: wrapped) { if !whole { model.setCommandCut(item, $0) } })
                    .accessibilityLabel(command)
                    .font(.footnote.monospaced())
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(.vertical, 8)
                    .padding(.horizontal, 10)
                    .background(.fill.tertiary, in: RoundedRectangle(cornerRadius: 12))
            }
            if showsReason, let reason = approval.reason {
                ApprovalReason(reason: reason)
            }
            if approval.options.isEmpty, !approval.choices.isEmpty {
                QuestionText(snippet: approval.snippet)
            } else if !approval.snippet.isEmpty, whole {
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
            if Countdown.shows(untilMs: approval.expiresAtMs, now: context.date) {
                countdown(now: context.date)
            }
        }
    }

    private func countdown(now: Date) -> some View {
        Label(Countdown.string(untilMs: approval.expiresAtMs, now: now), systemImage: "timer")
            .labelStyle(.titleAndIcon)
            .font(.footnote.weight(.semibold).monospacedDigit())
            .foregroundStyle(approval.expiresAtMs < now.unixMs + 60_000 ? AgentState.blocked.color : .metadata)
            .padding(.horizontal, 6)
            .padding(.vertical, 2)
            .padding(2)
            .fixedSize()
            // As tall as Review's glass beside it on the card and the band.
            .frame(maxHeight: .infinity)
            // The dimming color behind the glass, as on Review: the glass alone left the digits at 3.7:1 over the dark
            // terminal, and the last minute's red reads 4.6:1 there on the landscape band, its lowest.
            .glassEffect(.regular, in: .capsule)
            .background(SessionBarDim(), in: .capsule)
    }
}

struct ApprovalNotice: View {
    let notice: String

    var body: some View {
        Label(notice, systemImage: "exclamationmark.triangle")
            .font(.footnote)
            .foregroundStyle(.warning)
            .composerNoteGlass()
            .frame(maxWidth: .infinity, alignment: .leading)
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
        if model.noting.contains(item.id) {
            HStack(spacing: 8) {
                let placeholder = "Note for \(item.approval.agentLabel)"
                TextField(placeholder, text: model.draft(item), prompt: Text(placeholder).foregroundStyle(.metadata))
                    .font(.footnote)
                    .submitLabel(.done)
                    .focused(field, equals: "note")
                    .panelField()
                Button("Remove note", systemImage: "xmark.circle.fill") { model.toggleNote(item) }
                    .labelStyle(.iconOnly)
                    .buttonStyle(.plain)
                    .foregroundStyle(.secondary)
            }
            .id("note")
            .disabled(model.steps[item.id] != nil)
        } else {
            // On the band's glass over the dark terminal the primary color at 60% read 4.2:1 and the gold accent 3.8:1, so
            // the label takes the primary color at 75% and only the symbol the accent.
            Button {
                model.toggleNote(item)
                field.wrappedValue = "note"
            } label: {
                Label {
                    Text("Add note").foregroundStyle(Color.primary.opacity(0.75))
                } icon: {
                    Image(systemName: "text.bubble").foregroundStyle(.tint)
                }
                // The half-height sheet draws at 96%, where 44 points measured 42.2.
                .frame(minHeight: 46)
                .contentShape(Rectangle())
            }
            .font(.caption)
            .disabled(model.steps[item.id] != nil)
            // The tap target reaches 7 points into the 8-point gaps around its row, which keeps the row compact.
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.vertical, -7)
        }
    }
}

extension View {
    /// The command and output panels' fill and shape: the rounded border style drew a black box on the dark sheet.
    fileprivate func panelField() -> some View {
        padding(.vertical, 8)
            .padding(.horizontal, 10)
            .background(.fill.tertiary, in: RoundedRectangle(cornerRadius: 12))
    }
}

private struct FeedbackField: View {
    let model: ApprovalsModel
    let item: ApprovalItem
    let field: FocusState<String?>.Binding
    let sendAnswer: (String) async throws -> Void

    var body: some View {
        HStack(spacing: 8) {
            TextField("Answer this prompt", text: model.draft(item), prompt: Text("Answer this prompt").foregroundStyle(.metadata))
                .font(.footnote)
                .submitLabel(.send)
                .focused(field, equals: "answer")
                .panelField()
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
