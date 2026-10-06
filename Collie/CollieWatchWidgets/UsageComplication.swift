import SwiftUI
import WidgetKit

/// The five-hour plan usage the watch app last received. Plan usage rather than context:
/// one figure per subscription, where context would need one agent picked arbitrarily.
@main
struct UsageComplication: Widget {
    var body: some WidgetConfiguration {
        StaticConfiguration(kind: "CollieUsage", provider: UsageProvider()) { entry in
            UsageRing(used: entry.used)
                .containerBackground(for: .widget) { AccessoryWidgetBackground() }
        }
        .configurationDisplayName("Claude usage")
        .supportedFamilies([.accessoryCircular])
    }
}

struct UsageEntry: TimelineEntry {
    let date: Date
    let used: UInt8?
}

/// The watch app reloads the timeline when the figure changes; the second entry clears it at the reset.
struct UsageProvider: TimelineProvider {
    func placeholder(in context: Context) -> UsageEntry {
        UsageEntry(date: .now, used: 42)
    }

    func getSnapshot(in context: Context, completion: @escaping (UsageEntry) -> Void) {
        completion(placeholder(in: context))
    }

    func getTimeline(in context: Context, completion: @escaping (Timeline<UsageEntry>) -> Void) {
        let usage = WatchUsage.load()
        var entries = [UsageEntry(date: .now, used: usage?.fiveHour(now: .now))]
        if let reset = usage?.fiveHourResetsAtMs.map({ Date(timeIntervalSince1970: TimeInterval($0) / 1000) }), reset > .now {
            entries.append(UsageEntry(date: reset, used: nil))
        }
        completion(Timeline(entries: entries, policy: .never))
    }
}

private struct UsageRing: View {
    let used: UInt8?

    var body: some View {
        Gauge(value: Double(used ?? 0), in: 0...100) {
            Text("5h")
        } currentValueLabel: {
            Text(used.map { "\($0)" } ?? "--")
        }
        .gaugeStyle(.accessoryCircularCapacity)
    }
}
