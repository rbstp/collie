import SwiftUI
import WidgetKit

struct UsageGauges: View {
    let usage: WatchUsage?
    let now: Date

    var body: some View {
        let windows = usage?.windows(now: now) ?? [nil, nil, nil]
        GeometryReader { geometry in
            let diameter = max(0, min((geometry.size.width - 4) / 3, geometry.size.height - 16))
            HStack(spacing: 2) {
                UsageGauge(logo: "Claude", label: "Claude 5-hour", window: windows[0], now: now, diameter: diameter)
                UsageGauge(logo: "Claude", label: "Claude weekly", window: windows[1], now: now, diameter: diameter)
                UsageGauge(logo: "Codex", label: "Codex monthly", window: windows[2], now: now, diameter: diameter)
            }
            .frame(width: geometry.size.width, height: geometry.size.height)
        }
    }
}

private struct UsageGauge: View {
    let logo: String
    let label: String
    let window: WatchUsage.Window?
    let now: Date
    let diameter: CGFloat

    var body: some View {
        VStack(spacing: 2) {
            UsageDial(logo: logo, window: window, now: now, showsPercentage: false)
                .controlSize(.extraLarge)
                .frame(width: diameter, height: diameter)
            HStack(spacing: 2) {
                if let window {
                    if window.isStale(now: now) {
                        Circle().fill(.orange).frame(width: 3, height: 3)
                    }
                    Text(Duration.seconds(max(0, window.interval.upperBound.timeIntervalSince(now))),
                         format: .units(allowed: [.days, .hours, .minutes], width: .narrow, maximumUnitCount: 2))
                } else {
                    Text("No data")
                }
            }
            .font(.system(size: 11, weight: .bold, design: .rounded))
            .foregroundStyle(.primary)
            .frame(height: 14)
        }
        .frame(width: diameter)
        .lineLimit(1)
        .minimumScaleFactor(0.8)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(label)
        .accessibilityValue(window.map {
            "\($0.used) percent used. \(Int(($0.elapsed(now: now) * 100).rounded())) percent of the window elapsed. Resets \($0.interval.upperBound.formatted(date: .abbreviated, time: .shortened))"
                + ($0.isStale(now: now) ? ". Usage may be outdated" : "")
        } ?? "Usage unavailable")
    }
}
