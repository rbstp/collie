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
                .onChange(of: model.items) { _, _ in
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
            if !approval.snippet.isEmpty {
                Text(verbatim: approval.snippet)
                    .font(.caption.monospaced())
                    .foregroundStyle(.secondary)
                    .lineLimit(expanded ? nil : 8)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(8)
                    .background(.fill.tertiary, in: RoundedRectangle(cornerRadius: 8))
            }
            TimelineView(.periodic(from: .now, by: 1)) { context in
                let expired = approval.expiresAtMs <= UInt64(context.date.timeIntervalSince1970 * 1000)
                if approval.options.isEmpty {
                    ChoiceButtons(model: model, item: item, expired: expired)
                } else {
                    DecisionButtons(model: model, item: item, expired: expired)
                }
            }
        }
        .padding(.vertical, 4)
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
            }
        }
        .disabled(expired || model.steps[item.id] != nil)
    }
}

/// The menu on the Mac, for prompts collied offers no Approve/Deny for; each pick goes through `approval.decide`.
private struct ChoiceButtons: View {
    let model: ApprovalsModel
    let item: ApprovalItem
    let expired: Bool

    var body: some View {
        VStack(spacing: 6) {
            ForEach(item.approval.choices, id: \.index) { choice in
                let decision = ApprovalDecision.choose(choice: choice.index)
                Button {
                    Task { await model.decide(item, decision) }
                } label: {
                    HStack(alignment: .firstTextBaseline, spacing: 8) {
                        Text(verbatim: "\(Int(choice.index) + 1).").monospacedDigit()
                        Text(verbatim: choice.label).multilineTextAlignment(.leading)
                        Spacer(minLength: 0)
                        if model.steps[item.id] == .sending(decision) {
                            ProgressView()
                        } else if choice.current {
                            Image(systemName: "arrowtriangle.left.fill")
                                .font(.caption2)
                                .accessibilityLabel("Under the cursor on the Mac")
                        }
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                }
                .buttonStyle(.bordered)
            }
        }
        .disabled(expired || model.steps[item.id] != nil)
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
