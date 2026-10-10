import SwiftUI
import WidgetKit

@main
struct UsageComplication: Widget {
    var body: some WidgetConfiguration {
        StaticConfiguration(kind: WatchUsage.widgetKind, provider: UsageProvider()) { entry in
            UsageWidgetView(entry: entry)
        }
        .configurationDisplayName("Plan usage")
        .supportedFamilies([.accessoryCircular, .accessoryRectangular])
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
                codexUsed: 90, codexResetsAtMs: month.end.unixMs,
                claudeRecordedMs: now.unixMs, codexRecordedMs: now.unixMs
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

private struct UsageWidgetView: View {
    let entry: UsageEntry
    @Environment(\.widgetFamily) private var family

    var body: some View {
        Group {
            if family == .accessoryRectangular {
                UsageBars(usage: entry.usage, now: entry.date)
            } else {
                UsageRings(usage: entry.usage, now: entry.date)
            }
        }
        .widgetURL(WatchUsage.detailsURL)
        .containerBackground(for: .widget) {
            if family == .accessoryRectangular {
                Color.black
            } else {
                AccessoryWidgetBackground()
            }
        }
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
                    let strokeWidth = diameter * 0.1
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
                            Circle().stroke(.secondary.opacity(0.25), style: StrokeStyle(lineWidth: strokeWidth, dash: [strokeWidth, strokeWidth]))
                                .padding(strokeWidth / 2)
                        }
                    }
                    .frame(width: diameter, height: diameter)
                    if let window = windows[index] {
                        Capsule().fill(.white)
                            .frame(width: strokeWidth * 0.2, height: strokeWidth)
                            .offset(y: -(diameter - strokeWidth) / 2)
                            .rotationEffect(.degrees(window.remainingUsage * 360))
                    }
                }
                if windows.allSatisfy({ $0 == nil }) {
                    Text("--").font(.system(size: size * 0.2)).foregroundStyle(.secondary)
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
