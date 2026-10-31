import AppIntents
import SwiftUI
import WidgetKit

@main
struct UsageComplication: Widget {
    var body: some WidgetConfiguration {
        AppIntentConfiguration(kind: WatchUsage.widgetKind, intent: UsageAgentsIntent.self, provider: UsageProvider()) { entry in
            UsageWidgetView(entry: entry)
        }
        .configurationDisplayName("Plan usage")
        .supportedFamilies([.accessoryCircular, .accessoryRectangular])
    }
}

struct UsageAgentsIntent: WidgetConfigurationIntent {
    static let title: LocalizedStringResource = "Agents"
    static let description = IntentDescription("Choose the agents this widget shows.")

    @Parameter(title: "Claude Code", default: true) var claude: Bool
    @Parameter(title: "Codex", default: true) var codex: Bool
    @Parameter(title: "Copilot", default: true) var copilot: Bool

    var agents: UsageAgents {
        claude || codex || copilot ? UsageAgents(claude: claude, codex: codex, copilot: copilot) : UsageAgents()
    }
}

struct UsageEntry: TimelineEntry {
    let date: Date
    let usage: WatchUsage?
    var agents = UsageAgents()
}

struct UsageProvider: AppIntentTimelineProvider {
    // Empty, so watchOS 26 lets the owner edit the intent instead of picking a preset.
    func recommendations() -> [AppIntentRecommendation<UsageAgentsIntent>] { [] }

    func placeholder(in context: Context) -> UsageEntry {
        let now = Date.now
        let month = Calendar(identifier: .gregorian).dateInterval(of: .month, for: now)!
        return UsageEntry(
            date: now,
            usage: WatchUsage(
                fiveHourUsed: 42, fiveHourResetsAtMs: now.addingTimeInterval(10_800).unixMs,
                sevenDayUsed: 73, sevenDayResetsAtMs: now.addingTimeInterval(4 * 24 * 3600).unixMs,
                codexUsed: 90, codexResetsAtMs: month.end.unixMs,
                claudeRecordedMs: now.unixMs, codexRecordedMs: now.unixMs,
                copilotUsed: 55, copilotResetsAtMs: month.end.unixMs, copilotRecordedMs: now.unixMs
            )
        )
    }

    func snapshot(for configuration: UsageAgentsIntent, in context: Context) async -> UsageEntry {
        var entry = context.isPreview ? placeholder(in: context) : UsageEntry(date: .now, usage: WatchUsage.load())
        entry.agents = configuration.agents
        return entry
    }

    func timeline(for configuration: UsageAgentsIntent, in context: Context) async -> Timeline<UsageEntry> {
        let now = Date.now
        let usage = WatchUsage.load()
        let entries = (usage?.timelineDates(now: now) ?? [now]).map { UsageEntry(date: $0, usage: usage, agents: configuration.agents) }
        return Timeline(entries: entries, policy: entries.count > 1 ? .atEnd : .after(now.addingTimeInterval(15 * 60)))
    }
}

private struct UsageWidgetView: View {
    let entry: UsageEntry
    @Environment(\.widgetFamily) private var family

    var body: some View {
        Group {
            if family == .accessoryRectangular {
                UsageGauges(usage: entry.usage, now: entry.date, agents: entry.agents)
            } else {
                UsageRings(usage: entry.usage, now: entry.date, agents: entry.agents)
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
    let agents: UsageAgents
    private let labels = ["Claude 5-hour", "Claude weekly", "Codex monthly", "Copilot monthly"]

    var body: some View {
        let windows = usage?.windows(now: now) ?? [nil, nil, nil, nil]
        let shown = agents.windows
        GeometryReader { geometry in
            let size = min(geometry.size.width, geometry.size.height)
            ZStack {
                ForEach(Array(shown.enumerated()), id: \.element) { position, index in
                    let diameter = size * (1 - Double(position) * 0.22)
                    let strokeWidth = size * 0.075
                    Group {
                        if let window = windows[index] {
                            ZStack {
                                Circle().stroke(WatchUsage.usedColor(window.used).opacity(0.2), lineWidth: strokeWidth)
                                if window.used > 0 {
                                    Circle().trim(from: 0, to: window.usedFraction)
                                        .stroke(WatchUsage.usedColor(window.used).gradient, style: StrokeStyle(lineWidth: strokeWidth, lineCap: .round))
                                        .rotationEffect(.degrees(-90))
                                }
                            }
                            .padding(strokeWidth / 2)
                        } else {
                            Circle().stroke(.secondary.opacity(0.25), style: StrokeStyle(lineWidth: strokeWidth, dash: [strokeWidth, strokeWidth]))
                                .padding(strokeWidth / 2)
                        }
                    }
                    .widgetAccentable()
                    .frame(width: diameter, height: diameter)
                    if let window = windows[index] {
                        Capsule().fill(.white)
                            .frame(width: strokeWidth * 0.2, height: strokeWidth)
                            .offset(y: -(diameter - strokeWidth) / 2)
                            .rotationEffect(.degrees(window.elapsed(now: now) * 360))
                    }
                }
                if shown.allSatisfy({ windows[$0] == nil }) {
                    Text("--").font(.system(size: size * 0.2)).foregroundStyle(.secondary)
                }
            }
            .frame(width: geometry.size.width, height: geometry.size.height)
        }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(shown.map { index in
            let label = labels[index]
            return windows[index].map { "\(label), \($0.unlimited ? "unlimited" : "\($0.used) percent used"), \(Int(($0.elapsed(now: now) * 100).rounded())) percent of the window elapsed" } ?? "\(label), unavailable"
        }.joined(separator: ". "))
    }
}
