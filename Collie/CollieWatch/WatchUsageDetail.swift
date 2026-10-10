import SwiftUI
import WidgetKit

struct WatchUsageDetail: View {
    let model: WatchModel

    var body: some View {
        ScrollView {
            GlassEffectContainer(spacing: 8) {
                VStack(alignment: .leading, spacing: 10) {
                    ForEach(0..<3) { index in
                        TimelineView(.everyMinute) { context in
                            let windows = model.usage?.windows(now: context.date) ?? [nil, nil, nil]
                            UsageDetailSection(
                                logo: index < 2 ? "Claude" : "Codex",
                                period: ["5-hour window", "Weekly", "Monthly"][index],
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
                .padding(.horizontal, 8)
            }
        }
        .navigationTitle("Usage")
    }
}

private struct UsageDetailSection: View {
    let logo: String
    let period: String
    let window: WatchUsage.Window?
    let now: Date

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack(spacing: 8) {
                UsageLogo(name: logo).frame(width: 20, height: 20).accessibilityHidden(true)
                VStack(alignment: .leading, spacing: 1) {
                    Text(logo).font(.system(.headline, design: .rounded))
                    Text(period).font(.caption2).foregroundStyle(.secondary)
                }
            }
            VStack(alignment: .leading, spacing: 6) {
                HStack(alignment: .firstTextBaseline, spacing: 4) {
                    Text(window.map { "\($0.used)%" } ?? "--")
                        .font(.system(.largeTitle, design: .rounded).weight(.semibold))
                        .monospacedDigit()
                    Text(window == nil ? "Unavailable" : "used").font(.caption).foregroundStyle(.secondary)
                }
                if let window {
                    UsagePaceBar(window: window, now: now)
                        .padding(.vertical, 4)
                    HStack(spacing: 3) {
                        Text(.currentDate, format: .offset(
                            to: window.interval.upperBound, allowedFields: [.day, .hour, .minute], maxFieldCount: 2, sign: .never
                        ))
                        Text("left")
                    }
                    Text("Resets \(window.interval.upperBound.formatted(.dateTime.month(.abbreviated).day().hour().minute()))")
                        .foregroundStyle(.secondary)
                    if let recordedAt = window.recordedAt {
                        Text("Updated \(recordedAt.formatted(.relative(presentation: .numeric)))")
                            .font(.caption2)
                            .foregroundStyle(.tertiary)
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
        }
        .padding(14)
        .frame(maxWidth: .infinity, alignment: .leading)
        .glassEffect(.regular, in: RoundedRectangle(cornerRadius: 20, style: .continuous))
    }
}
