import SwiftUI
import WidgetKit

struct UsagePaceBar: View {
    let window: WatchUsage.Window
    let now: Date

    var body: some View {
        GeometryReader { geometry in
            ZStack(alignment: .leading) {
                Capsule().fill(WatchUsage.usedColor(window.used).opacity(0.18)).widgetAccentable()
                if window.used > 0 {
                    Capsule().fill(WatchUsage.usedColor(window.used).gradient)
                        .frame(width: geometry.size.width * window.usedFraction)
                        .widgetAccentable()
                }
                Capsule().fill(.white)
                    .frame(width: 1.5, height: 6)
                    .position(x: min(max(geometry.size.width * window.elapsed(now: now), 0.75), geometry.size.width - 0.75), y: geometry.size.height / 2)
            }
        }
        .frame(height: 4)
        .accessibilityHidden(true)
    }
}
