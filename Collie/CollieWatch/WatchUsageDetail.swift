import SwiftUI
import WidgetKit

struct WatchUsageDetail: View {
    let model: WatchModel

    var body: some View {
        List {
            ForEach(0..<3) { index in
                TimelineView(.everyMinute) { context in
                    let window = model.usage?.windows(now: context.date)[index]
                    let logo = index < 2 ? "Claude" : "Codex"
                    let period = ["5-hour", "Weekly", "Monthly"][index]
                    let shortPeriod = ["5h", "7d", "1m"][index]
                    NavigationLink {
                        TimelineView(.everyMinute) { detail in
                            UsageWindowDetail(logo: logo, period: period, shortPeriod: shortPeriod,
                                              window: model.usage?.windows(now: detail.date)[index], now: detail.date)
                        }
                    } label: {
                        HStack(spacing: 10) {
                            UsageDial(logo: logo, window: window, now: context.date, showsPercentage: false)
                                .frame(width: 54, height: 54)
                                .accessibilityHidden(true)
                            VStack(alignment: .leading, spacing: 3) {
                                Text("\(period) · \(window.map { "\($0.used)%" } ?? "--")")
                                    .font(.headline)
                                if let window {
                                    Text("\(window.remaining(now: context.date)) left")
                                } else {
                                    Text("No data")
                                }
                            }
                            .font(.footnote)
                        }
                        .padding(.vertical, 4)
                        .accessibilityElement(children: .ignore)
                        .accessibilityLabel("\(logo) \(period)")
                        .accessibilityValue(window?.accessibilityValue(now: context.date) ?? "Usage unavailable")
                    }
                }
            }
            Section("Phone sync") {
                if model.refreshing {
                    ProgressView("Syncing")
                } else if let lastSyncedAt = model.lastSyncedAt {
                    Text(lastSyncedAt.formatted(date: .abbreviated, time: .shortened))
                } else {
                    Text("Not synced yet").foregroundStyle(.secondary)
                }
                if model.refreshFailed {
                    Text("Couldn't refresh from the iPhone").foregroundStyle(.secondary)
                }
            }
            .font(.footnote)
        }
        .navigationTitle("Usage")
    }
}

private struct UsageWindowDetail: View {
    let logo: String
    let period: String
    let shortPeriod: String
    let window: WatchUsage.Window?
    let now: Date

    var body: some View {
        ScrollView {
            VStack(spacing: 12) {
                UsageDial(logo: logo, period: shortPeriod, window: window, now: now)
                    .scaleEffect(1.6)
                    .frame(width: 88, height: 88)
                    .accessibilityLabel(window.map { "\($0.used) percent used" } ?? "Usage unavailable")
                if let window {
                    let remaining = Text(.currentDate, format: .offset(
                        to: window.interval.upperBound, allowedFields: [.day, .hour, .minute], maxFieldCount: 2, sign: .never
                    ))
                    Text("\(remaining) left")
                    Text("Resets \(window.interval.upperBound.formatted(.dateTime.month(.abbreviated).day().hour().minute()))")
                        .foregroundStyle(.secondary)
                    Text("\(Int((window.elapsed(now: now) * 100).rounded()))% of window elapsed")
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
                    Text("Waiting for a current reading").foregroundStyle(.secondary)
                }
            }
            .font(.footnote)
            .multilineTextAlignment(.center)
            .frame(maxWidth: .infinity)
            .padding(.horizontal, 8)
        }
        .navigationTitle("\(logo) \(period)")
    }
}
