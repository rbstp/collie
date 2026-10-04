import ActivityKit
import AppIntents
import SwiftUI
import WidgetKit

@main
struct CollieWidgets: WidgetBundle {
    var body: some Widget {
        AgentLiveActivity()
    }
}

struct AgentLiveActivity: Widget {
    var body: some WidgetConfiguration {
        ActivityConfiguration(for: AgentActivityAttributes.self) { context in
            LockScreenView(context: context)
                .activityBackgroundTint(.black.opacity(0.85))
                .activitySystemActionForegroundColor(.white)
                .widgetURL(link(context))
        } dynamicIsland: { context in
            let state = context.state
            return DynamicIsland {
                DynamicIslandExpandedRegion(.leading) {
                    StatusPill(status: state.status).padding(.leading, 4)
                }
                DynamicIslandExpandedRegion(.trailing) {
                    ElapsedText(since: state.statusSince)
                        .font(.caption.monospacedDigit())
                        .foregroundStyle(.secondary)
                        .padding(.trailing, 4)
                }
                DynamicIslandExpandedRegion(.center) {
                    Text(state.title).font(.headline).lineLimit(1)
                }
                DynamicIslandExpandedRegion(.bottom) {
                    VStack(alignment: .leading, spacing: 6) {
                        // The island is too short for both: the command matters more.
                        if state.pendingApproval == nil {
                            Text(subtitle(context)).font(.caption).foregroundStyle(.secondary).lineLimit(1)
                        }
                        Approval(context: context)
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(.horizontal, 4)
                }
            } compactLeading: {
                Image(systemName: state.status.symbol).foregroundStyle(state.status.color)
            } compactTrailing: {
                Text(state.status.label).font(.caption2.bold()).foregroundStyle(state.status.color)
            } minimal: {
                Circle().fill(state.status.color).frame(width: 10, height: 10)
            }
            .widgetURL(link(context))
            .keylineTint(state.status.color)
        }
    }
}

private func link(_ context: ActivityViewContext<AgentActivityAttributes>) -> URL? {
    AgentLink(machineId: context.attributes.machineId, terminalId: context.attributes.terminalId)?.url
}

private func subtitle(_ context: ActivityViewContext<AgentActivityAttributes>) -> String {
    [context.state.workspace, context.attributes.machineLabel].compactMap { $0 }.filter { !$0.isEmpty }.joined(separator: " · ")
}

private struct LockScreenView: View {
    let context: ActivityViewContext<AgentActivityAttributes>

    var body: some View {
        let state = context.state
        VStack(alignment: .leading, spacing: 6) {
            HStack(spacing: 8) {
                StatusPill(status: state.status)
                Text(state.title).font(.headline).lineLimit(1)
                Spacer(minLength: 4)
                if context.isStale {
                    Image(systemName: "clock.badge.exclamationmark").foregroundStyle(.secondary)
                }
                ElapsedText(since: state.statusSince)
                    .font(.subheadline.monospacedDigit())
                    .foregroundStyle(.secondary)
            }
            Text(subtitle(context)).font(.caption).foregroundStyle(.secondary).lineLimit(1)
            Approval(context: context)
        }
        .padding(16)
        .environment(\.colorScheme, .dark)
    }
}

private struct StatusPill: View {
    let status: AgentActivityStatus

    var body: some View {
        Text(status.label)
            .font(.caption.bold())
            .padding(.horizontal, 8)
            .padding(.vertical, 3)
            .background(status.color.opacity(0.18), in: Capsule())
            .foregroundStyle(status.color)
    }
}

/// The pending approval's command, decrypted here with the Mac's notification key, and the
/// buttons that decide it. Without a command that decrypts, only "Approval needed".
private struct Approval: View {
    let context: ActivityViewContext<AgentActivityAttributes>

    var body: some View {
        let state = context.state
        // Buttons only for an approval whose command opened under the Mac's key with its id as
        // AAD: an id Apple or the APNs key holder pushed in clear must not become decidable here.
        if let approvalId = state.pendingApproval, let nodeId = context.attributes.nodeId,
            let command = state.command(key: NotificationKey.load(nodeId: nodeId))
        {
            VStack(alignment: .leading, spacing: 8) {
                Text(command)
                    .font(.caption.monospaced())
                    .lineLimit(3)
                    .truncationMode(.tail)
                    .privacySensitive()
                if let progress = state.progress {
                    Text(progress).font(.subheadline.bold()).foregroundStyle(.secondary)
                } else {
                    HStack(spacing: 8) {
                        Button(intent: DecideApprovalIntent(nodeId: nodeId, approvalId: approvalId, decision: .deny)) {
                            Text("Deny").frame(maxWidth: .infinity)
                        }
                        .tint(.red)
                        Button(intent: DecideApprovalIntent(nodeId: nodeId, approvalId: approvalId, decision: .approve)) {
                            Text("Approve").frame(maxWidth: .infinity)
                        }
                        .tint(.green)
                    }
                    .buttonStyle(.borderedProminent)
                    .font(.subheadline.bold())
                }
            }
        } else if state.status == .blocked {
            ApprovalNeeded()
        }
    }
}

/// Never runs: iOS runs a LiveActivityIntent in the app, which has the real `perform`.
extension DecideApprovalIntent {
    func perform() async throws -> some IntentResult { .result() }
}

private struct ApprovalNeeded: View {
    var body: some View {
        Label("Approval needed", systemImage: "exclamationmark.shield.fill")
            .font(.subheadline.bold())
            .foregroundStyle(.red)
    }
}

private struct ElapsedText: View {
    let since: Date

    var body: some View {
        Text(timerInterval: since...Date.distantFuture, countsDown: false)
            .multilineTextAlignment(.trailing)
            .frame(maxWidth: 64, alignment: .trailing)
    }
}
