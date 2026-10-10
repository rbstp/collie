import SwiftUI
import WidgetKit

struct UsageGauges: View {
    let usage: WatchUsage?
    let now: Date

    var body: some View {
        let windows = usage?.windows(now: now) ?? [nil, nil, nil]
        GeometryReader { geometry in
            let diameter = max(0, min((geometry.size.width - 12) / 3, geometry.size.height - 25))
            HStack(spacing: 6) {
                UsageGauge(logo: "Claude", period: "5h", label: "Claude 5-hour", window: windows[0], now: now, diameter: diameter)
                UsageGauge(logo: "Claude", period: "7d", label: "Claude weekly", window: windows[1], now: now, diameter: diameter)
                UsageGauge(logo: "Codex", period: "1m", label: "Codex monthly", window: windows[2], now: now, diameter: diameter)
            }
            .frame(width: geometry.size.width, height: geometry.size.height)
        }
    }
}

private struct UsageGauge: View {
    let logo: String
    let period: String
    let label: String
    let window: WatchUsage.Window?
    let now: Date
    let diameter: CGFloat

    var body: some View {
        VStack(spacing: 3) {
            Gauge(value: window?.usedFraction ?? 0) {
                Text(period).font(.system(size: 10, weight: .semibold, design: .rounded))
            } currentValueLabel: {
                VStack(spacing: 0) {
                    UsageLogo(name: logo).frame(width: 13, height: 13)
                    Text(window.map { "\($0.used)%" } ?? "--")
                        .font(.system(size: 11, weight: .semibold, design: .rounded))
                        .monospacedDigit()
                }
            }
            .gaugeStyle(.accessoryCircular)
            .fixedSize()
            .visualEffect { content, geometry in
                content.scaleEffect(min(1, diameter / geometry.size.width))
            }
            .frame(width: diameter, height: diameter)
            .tint(window.map { WatchUsage.usedColor($0.used) } ?? .secondary.opacity(0.3))
            .widgetAccentable()
            VStack(spacing: 0) {
                if let window {
                    HStack(spacing: 2) {
                        if window.isStale(now: now) {
                            Circle().fill(.orange).frame(width: 3, height: 3)
                        }
                        Text(window.interval.upperBound, format: .dateTime.month(.abbreviated).day())
                    }
                    Text(window.interval.upperBound, format: .dateTime.hour().minute())
                } else {
                    Text("No data")
                }
            }
            .font(.system(size: 9, design: .rounded))
            .foregroundStyle(.secondary)
            .frame(height: 22)
        }
        .frame(width: diameter)
        .lineLimit(1)
        .minimumScaleFactor(0.8)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(label)
        .accessibilityValue(window.map {
            "\($0.used) percent used. Resets \($0.interval.upperBound.formatted(date: .abbreviated, time: .shortened))"
                + ($0.isStale(now: now) ? ". Usage may be outdated" : "")
        } ?? "Usage unavailable")
    }
}
