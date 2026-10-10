import SwiftUI
import WidgetKit

struct UsageBars: View {
    let usage: WatchUsage?
    let now: Date

    var body: some View {
        let windows = usage?.windows(now: now) ?? [nil, nil, nil]
        VStack(spacing: 8) {
            UsageBarRow(logo: "Claude", period: "5h", label: "Claude 5-hour", window: windows[0], now: now)
            UsageBarRow(logo: "Claude", period: "7d", label: "Claude weekly", window: windows[1], now: now)
            UsageBarRow(logo: "Codex", period: "1mo", label: "Codex monthly", window: windows[2], now: now)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}

private struct UsageBarRow: View {
    let logo: String
    let period: String
    let label: String
    let window: WatchUsage.Window?
    let now: Date

    var body: some View {
        HStack(spacing: 6) {
            UsageLogo(name: logo).frame(width: 14, height: 14).accessibilityHidden(true)
            VStack(spacing: 4) {
                HStack(alignment: .firstTextBaseline, spacing: 4) {
                    Text(period)
                        .font(.system(size: 10, weight: .medium, design: .rounded))
                        .foregroundStyle(logo == "Claude" ? Color(red: 0.80, green: 0.47, blue: 0.36) : .primary)
                        .accessibilityLabel(label)
                    Spacer(minLength: 0)
                    Group {
                        if let window {
                            Text(.currentDate, format: .offset(
                                to: window.interval.upperBound, allowedFields: [.day, .hour, .minute], maxFieldCount: 2, sign: .never
                            ))
                            .accessibilityHint("Time until reset")
                        } else {
                            Text("Unavailable")
                        }
                    }
                    .font(.system(size: 9, design: .rounded))
                    .foregroundStyle(.secondary)
                    HStack(spacing: 3) {
                        if let window, window.isStale(now: now) {
                            Circle().fill(.orange).frame(width: 3, height: 3).accessibilityHidden(true)
                        }
                        Text(window.map { "\($0.used)%" } ?? "--")
                    }
                    .font(.system(size: 13, weight: .semibold, design: .rounded))
                    .monospacedDigit()
                    .layoutPriority(1)
                    .accessibilityLabel(window.map {
                        "\($0.used) percent used, \(Int(($0.elapsed(now: now) * 100).rounded())) percent of the window elapsed"
                            + ($0.isStale(now: now) ? ", usage may be outdated" : "")
                    } ?? "Usage unavailable")
                }
                if let window {
                    UsagePaceBar(window: window, now: now)
                } else {
                    Capsule().fill(.quaternary).frame(height: 4)
                }
            }
        }
        .lineLimit(1)
        .minimumScaleFactor(0.8)
        .accessibilityElement(children: .combine)
    }
}
