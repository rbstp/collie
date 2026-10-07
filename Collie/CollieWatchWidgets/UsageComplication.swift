import SwiftUI
import WidgetKit

/// The five-hour plan usage the watch app last received, inside a ring of the time left until it resets.
/// Plan usage rather than context: one figure per subscription, where context would need one agent picked arbitrarily.
@main
struct UsageComplication: Widget {
    var body: some WidgetConfiguration {
        StaticConfiguration(kind: WatchUsage.widgetKind, provider: UsageProvider()) { entry in
            UsageRing(usage: entry.usage, now: entry.date)
                .containerBackground(for: .widget) { AccessoryWidgetBackground() }
        }
        .configurationDisplayName("Claude usage")
        .supportedFamilies([.accessoryCircular])
    }
}

struct UsageEntry: TimelineEntry {
    let date: Date
    let usage: WatchUsage?
}

/// The watch app reloads the timeline when the figure changes; the ring runs down to the reset by itself, and
/// the entries change its color.
struct UsageProvider: TimelineProvider {
    func placeholder(in context: Context) -> UsageEntry {
        let reset = UInt64((Date.now.timeIntervalSince1970 + 10_800) * 1000)
        return UsageEntry(date: .now, usage: WatchUsage(fiveHourUsed: 42, fiveHourResetsAtMs: reset))
    }

    func getSnapshot(in context: Context, completion: @escaping (UsageEntry) -> Void) {
        completion(placeholder(in: context))
    }

    func getTimeline(in context: Context, completion: @escaping (Timeline<UsageEntry>) -> Void) {
        let usage = WatchUsage.load()
        let entries = (usage?.timelineDates(now: .now) ?? [.now]).map { UsageEntry(date: $0, usage: usage) }
        completion(Timeline(entries: entries, policy: entries.count > 1 ? .atEnd : .never))
    }
}

private struct UsageRing: View {
    let usage: WatchUsage?
    let now: Date

    var body: some View {
        let used = usage?.fiveHour(now: now)
        let left = usage?.fiveHourSecondsLeft(now: now)
        let reset = now.addingTimeInterval(left ?? 0)
        ProgressView(timerInterval: reset.addingTimeInterval(-WatchUsage.fiveHourLength)...reset, countsDown: true) {
            Text("5h")
        } currentValueLabel: {
            Text(used.map { "\($0)" } ?? "--")
                .foregroundStyle(used.map(WatchUsage.usedColor) ?? .primary)
        }
        .progressViewStyle(.circular)
        .tint(left.map(WatchUsage.ringColor))
        .widgetAccentable()
    }
}
