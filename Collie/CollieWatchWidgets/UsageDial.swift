import SwiftUI

struct UsageDial: View {
    let logo: String
    var period: String? = nil
    let window: WatchUsage.Window?
    var showsPercentage = true

    var body: some View {
        Gauge(value: window?.usedFraction ?? 0) {
            if let period {
                HStack(spacing: 2) {
                    if showsPercentage {
                        UsageLogo(name: logo).frame(width: 11, height: 11)
                    }
                    Text(period).font(.system(size: 10, weight: .semibold, design: .rounded))
                }
            }
        } currentValueLabel: {
            if showsPercentage {
                Text(window.map { "\($0.used)%" } ?? "--")
                    .font(.system(size: 16, weight: .semibold, design: .rounded))
                    .monospacedDigit()
            } else {
                UsageLogo(name: logo).frame(width: 20, height: 20)
            }
        }
        .gaugeStyle(.accessoryCircular)
        .tint(window.map { WatchUsage.usedColor($0.used) } ?? .secondary.opacity(0.3))
    }
}
