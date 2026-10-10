import SwiftUI
import WidgetKit

struct UsageBars: View {
    let usage: WatchUsage?
    let now: Date

    var body: some View {
        let windows = usage?.windows(now: now) ?? [nil, nil, nil]
        VStack(spacing: 5) {
            UsageBarRow(logo: "Claude", period: "5h", label: "Claude 5-hour", color: Self.claude, window: windows[0], now: now)
            UsageBarRow(logo: "Claude", period: "7d", label: "Claude weekly", color: Self.claude, window: windows[1], now: now)
            UsageBarRow(logo: "Codex", period: "1mo", label: "Codex monthly", color: .white, window: windows[2], now: now)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }

    private static let claude = Color(red: 0.80, green: 0.47, blue: 0.36)
}

private struct UsageBarRow: View {
    let logo: String
    let period: String
    let label: String
    let color: Color
    let window: WatchUsage.Window?
    let now: Date

    var body: some View {
        HStack(spacing: 5) {
            UsageLogo(name: logo)
                .frame(width: 15, height: 15)
                .accessibilityHidden(true)
            Text(period)
                .font(.system(size: 10, weight: .medium))
                .foregroundStyle(color)
                .frame(width: 23, alignment: .leading)
                .accessibilityLabel(label)
            VStack(alignment: .leading, spacing: 2) {
                if let window {
                    HStack(alignment: .firstTextBaseline, spacing: 2) {
                        Text(.currentDate, format: .offset(
                            to: window.interval.upperBound, allowedFields: [.day, .hour, .minute], maxFieldCount: 2, sign: .never
                        ))
                        Text("left")
                    }
                    .font(.system(size: 9))
                    .foregroundStyle(.secondary)
                    ProgressView(timerInterval: window.interval, countsDown: true) {
                        EmptyView()
                    } currentValueLabel: {
                        EmptyView()
                    }
                    .progressViewStyle(.linear)
                    .tint(WatchUsage.usedColor(window.used))
                    .overlay {
                        GeometryReader { geometry in
                            Capsule().fill(.white)
                                .frame(width: 1, height: 6)
                                .position(x: min(max(geometry.size.width * window.remainingUsage, 0.5), geometry.size.width - 0.5), y: geometry.size.height / 2)
                        }
                    }
                    .accessibilityHidden(true)
                } else {
                    Text("Unavailable")
                        .font(.system(size: 9))
                        .foregroundStyle(.secondary)
                    Capsule().fill(color.opacity(0.2)).frame(height: 4)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            Text(window.map { "\($0.used)%" } ?? "--")
                .font(.system(size: 12, weight: .semibold, design: .rounded))
                .monospacedDigit()
                .frame(width: 32, alignment: .trailing)
                .overlay(alignment: .topTrailing) {
                    if let window, window.isStale(now: now) {
                        Circle().fill(.orange).frame(width: 3, height: 3).offset(x: 3, y: -1)
                    }
                }
                .accessibilityLabel(window.map { "\($0.used) percent used" + ($0.isStale(now: now) ? ", usage may be outdated" : "") } ?? "Usage unavailable")
        }
        .lineLimit(1)
        .minimumScaleFactor(0.8)
        .accessibilityElement(children: .combine)
    }
}
