import SwiftUI
import WidgetKit

@main
struct UsageComplication: Widget {
    var body: some WidgetConfiguration {
        StaticConfiguration(kind: WatchUsage.widgetKind, provider: UsageProvider()) { entry in
            UsageRings(usage: entry.usage, now: entry.date)
                .containerBackground(for: .widget) { AccessoryWidgetBackground() }
        }
        .configurationDisplayName("Plan usage")
        .supportedFamilies([.accessoryCircular])
    }
}

struct UsageEntry: TimelineEntry {
    let date: Date
    let usage: WatchUsage?
}

struct UsageProvider: TimelineProvider {
    func placeholder(in context: Context) -> UsageEntry {
        let now = Date.now
        let month = Calendar(identifier: .gregorian).dateInterval(of: .month, for: now)!
        return UsageEntry(
            date: now,
            usage: WatchUsage(
                fiveHourUsed: 42, fiveHourResetsAtMs: now.addingTimeInterval(10_800).unixMs,
                sevenDayUsed: 73, sevenDayResetsAtMs: now.addingTimeInterval(4 * 24 * 3600).unixMs,
                codexUsed: 90, codexResetsAtMs: month.end.unixMs
            )
        )
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

private struct UsageRings: View {
    let usage: WatchUsage?
    let now: Date
    private let labels = ["Claude 5-hour", "Claude weekly", "Codex monthly"]

    var body: some View {
        let windows = usage?.windows(now: now) ?? [nil, nil, nil]
        GeometryReader { geometry in
            let size = min(geometry.size.width, geometry.size.height)
            ZStack {
                ForEach(0..<3) { index in
                    let diameter = size * (1 - Double(index) * 0.28)
                    Group {
                        if let window = windows[index] {
                            ProgressView(timerInterval: window.interval, countsDown: true) {
                                EmptyView()
                            } currentValueLabel: {
                                EmptyView()
                            }
                            .progressViewStyle(.circular)
                            .tint(WatchUsage.usedColor(window.used))
                        } else {
                            Circle().stroke(.secondary.opacity(0.25), lineWidth: diameter * 0.1)
                                .padding(diameter * 0.05)
                        }
                    }
                    .frame(width: diameter, height: diameter)
                }
            }
            .frame(width: geometry.size.width, height: geometry.size.height)
        }
        .widgetAccentable()
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(labels.enumerated().map { index, label in
            windows[index].map { "\(label), \($0.used) percent used" } ?? "\(label), unavailable"
        }.joined(separator: ". "))
    }
}
