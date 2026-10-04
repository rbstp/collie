import ActivityKit
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
                        Text(subtitle(context)).font(.caption).foregroundStyle(.secondary).lineLimit(1)
                        if state.status == .blocked {
                            ApprovalNeeded()
                        }
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
            if state.status == .blocked {
                ApprovalNeeded()
            }
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
