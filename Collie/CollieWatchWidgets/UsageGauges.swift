import SwiftUI
import WidgetKit

struct UsageAgents {
    var claude = true
    var codex = true
    var copilot = true

    var windows: [Int] {
        (claude ? [0, 1] : []) + (codex ? [2] : []) + (copilot ? [3] : [])
    }
}

struct UsageGauges: View {
    let usage: WatchUsage?
    let now: Date
    var agents = UsageAgents()

    var body: some View {
        let windows = usage?.windows(now: now) ?? [nil, nil, nil, nil]
        GeometryReader { geometry in
            let diameter = max(0, min((geometry.size.width - 4) / 3, geometry.size.height - 16))
            HStack(spacing: 2) {
                if agents.claude {
                    ClaudeGauge(fiveHour: windows[0], weekly: windows[1], now: now, diameter: diameter)
                }
                if agents.codex {
                    UsageGauge(logo: "Codex", label: "Codex monthly", window: windows[2], now: now, diameter: diameter)
                }
                if agents.copilot {
                    UsageGauge(logo: "Copilot", label: "Copilot monthly", window: windows[3], now: now, diameter: diameter)
                }
            }
            .frame(width: geometry.size.width, height: geometry.size.height)
        }
    }
}

private func spokenValue(_ window: WatchUsage.Window?, now: Date) -> String {
    window.map { $0.accessibilityValue(now: now) + ($0.isStale(now: now) ? ". Usage may be outdated" : "") } ?? "Usage unavailable"
}

private struct ClaudeGauge: View {
    let fiveHour: WatchUsage.Window?
    let weekly: WatchUsage.Window?
    let now: Date
    let diameter: CGFloat

    var body: some View {
        VStack(spacing: 2) {
            ZStack {
                UsageDial(logo: nil, window: fiveHour, now: now, showsPercentage: false, span: 0.5)
                    .controlSize(.extraLarge)
                    .frame(width: diameter, height: diameter)
                    .mask(alignment: .leading) { Rectangle().frame(width: diameter / 2 - 1) }
                    .accessibilityElement(children: .ignore)
                    .accessibilityLabel("Claude 5-hour")
                    .accessibilityValue(spokenValue(fiveHour, now: now))
                UsageDial(logo: nil, window: weekly, now: now, showsPercentage: false, span: 0.5)
                    .controlSize(.extraLarge)
                    .frame(width: diameter, height: diameter)
                    .scaleEffect(x: -1)
                    .mask(alignment: .trailing) { Rectangle().frame(width: diameter / 2 - 1) }
                    .accessibilityElement(children: .ignore)
                    .accessibilityLabel("Claude weekly")
                    .accessibilityValue(spokenValue(weekly, now: now))
                UsageLogo(name: "Claude").frame(width: 20, height: 20).accessibilityHidden(true)
            }
            HStack(spacing: 2) {
                if fiveHour == nil && weekly == nil {
                    Text("No data")
                } else {
                    fiveHour.map { $0.remaining(now: now, units: 1) } ?? Text("--")
                    Spacer(minLength: 0)
                    if (fiveHour ?? weekly)?.isStale(now: now) == true {
                        Circle().fill(.orange).frame(width: 3, height: 3)
                    }
                    Spacer(minLength: 0)
                    weekly.map { $0.remaining(now: now, units: 1) } ?? Text("--")
                }
            }
            .font(.system(size: 11, weight: .bold, design: .rounded))
            .foregroundStyle(.primary)
            .frame(height: 14)
            .accessibilityHidden(true)
        }
        .frame(width: diameter)
        .lineLimit(1)
        .minimumScaleFactor(0.8)
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
                    if window.unlimited {
                        Text("Unlimited")
                    } else {
                        window.remaining(now: now)
                    }
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
        .accessibilityValue(spokenValue(window, now: now))
    }
}
