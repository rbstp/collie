#if DEBUG
import SwiftUI

/// `COLLIE_DEMO_SCREENSHOTS=usage` or `=widget`: the documentation screenshots over demo usage, with no
/// iPhone. Debug builds only.
enum WatchDemo {
    static let mode = ProcessInfo.processInfo.environment["COLLIE_DEMO_SCREENSHOTS"]

    static var usage: WatchUsage {
        let now = Date.now
        let month = Calendar(identifier: .gregorian).dateInterval(of: .month, for: now)!
        return WatchUsage(
            fiveHourUsed: 38, fiveHourResetsAtMs: now.addingTimeInterval(7_980).unixMs,
            sevenDayUsed: 61, sevenDayResetsAtMs: now.addingTimeInterval(277_200).unixMs,
            codexUsed: 24, codexResetsAtMs: month.end.unixMs,
            claudeRecordedMs: now.unixMs, codexRecordedMs: now.unixMs
        )
    }
}

/// The rectangular complication's view as the Smart Stack shows it.
struct WatchWidgetDemo: View {
    var body: some View {
        UsageGauges(usage: WatchDemo.usage, now: .now)
            .frame(height: 84)
            .padding(10)
            .background(.white.opacity(0.14), in: .rect(cornerRadius: 22))
            .padding(.horizontal, 4)
    }
}
#endif
