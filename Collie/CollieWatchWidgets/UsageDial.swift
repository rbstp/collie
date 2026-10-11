import SwiftUI
import WidgetKit

struct UsageDial: View {
    let logo: String
    var period: String? = nil
    let window: WatchUsage.Window?
    let now: Date
    var showsPercentage = true

    var body: some View {
        let color = window.map { WatchUsage.usedColor($0.used) } ?? .secondary
        let fraction = window?.usedFraction ?? 0
        let end = window == nil ? 0 : min(fraction + 0.02, 1)
        let remainder = end < 1 ? color.opacity(0.2) : color
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
        .tint(Gradient(stops: [
            .init(color: color, location: 0),
            .init(color: color, location: end),
            .init(color: remainder, location: end),
            .init(color: remainder, location: 1),
        ]))
        .widgetAccentable()
        .overlay {
            if let window {
                GeometryReader { geometry in
                    let diameter = min(geometry.size.width, geometry.size.height)
                    Capsule().fill(.white)
                        .frame(width: 2, height: diameter * 0.16)
                        .offset(y: -diameter * 0.45)
                        .rotationEffect(.degrees(-135 + window.elapsed(now: now) * 270))
                        .frame(width: geometry.size.width, height: geometry.size.height)
                        .mask {
                            Rectangle().overlay {
                                Circle()
                                    .frame(width: diameter * 0.2, height: diameter * 0.2)
                                    .offset(y: -diameter * 0.45)
                                    .rotationEffect(.degrees(-135 + window.usedFraction * 270))
                                    .blendMode(.destinationOut)
                            }
                            .compositingGroup()
                        }
                }
                .allowsHitTesting(false)
                .accessibilityHidden(true)
            }
        }
    }
}
