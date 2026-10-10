import SwiftUI
import WidgetKit

struct WatchUsageDetail: View {
    let model: WatchModel

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                ForEach(0..<3) { index in
                    TimelineView(.everyMinute) { context in
                        let windows = model.usage?.windows(now: context.date) ?? [nil, nil, nil]
                        UsageDetailSection(
                            logo: index < 2 ? "Claude" : "Codex",
                            title: ["Claude · 5 hours", "Claude · Weekly", "Codex · Monthly"][index],
                            window: windows[index], now: context.date
                        )
                    }
                }
                VStack(alignment: .leading, spacing: 6) {
                    Text("Phone sync").font(.headline)
                    if model.refreshing {
                        ProgressView("Syncing")
                    } else if let lastSyncedAt = model.lastSyncedAt {
                        Text(lastSyncedAt.formatted(date: .abbreviated, time: .shortened))
                            .font(.caption)
                    } else {
                        Text("Not synced yet").foregroundStyle(.secondary)
                    }
                    if model.refreshFailed {
                        Text("Couldn't refresh from the iPhone").font(.caption).foregroundStyle(.secondary)
                    }
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.horizontal)
        }
        .navigationTitle("Plan usage")
    }
}

private struct UsageDetailSection: View {
    let logo: String
    let title: String
    let window: WatchUsage.Window?
    let now: Date

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack(spacing: 5) {
                UsageLogo(name: logo).frame(width: 14, height: 14).accessibilityHidden(true)
                Text(title).font(.headline)
            }
            VStack(alignment: .leading, spacing: 6) {
                HStack(alignment: .firstTextBaseline, spacing: 4) {
                    Text(window.map { "\($0.used)%" } ?? "--")
                        .font(.system(.title, design: .rounded).weight(.semibold))
                        .monospacedDigit()
                    Text(window == nil ? "Unavailable" : "used").font(.caption).foregroundStyle(.secondary)
                }
                if let window {
                    ProgressView(value: Double(window.used), total: 100)
                        .progressViewStyle(.linear)
                        .tint(WatchUsage.usedColor(window.used))
                        .accessibilityHidden(true)
                    HStack(spacing: 3) {
                        Text(.currentDate, format: .offset(
                            to: window.interval.upperBound, allowedFields: [.day, .hour, .minute], maxFieldCount: 2, sign: .never
                        ))
                        Text("left")
                    }
                    Text("Resets \(window.interval.upperBound.formatted(date: .abbreviated, time: .shortened))")
                        .foregroundStyle(.secondary)
                    if let recordedAt = window.recordedAt {
                        Text("Updated \(recordedAt.formatted(.relative(presentation: .numeric)))")
                            .foregroundStyle(.secondary)
                    }
                    if window.isStale(now: now) {
                        Label("Usage may be outdated", systemImage: "circle.fill")
                            .foregroundStyle(.orange)
                    }
                } else {
                    Capsule().fill(.quaternary).frame(height: 4)
                    Text("Waiting for a current reading").foregroundStyle(.secondary)
                }
            }
            .font(.caption)
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(12)
            .background(.quaternary, in: RoundedRectangle(cornerRadius: 14))
        }
    }
}
