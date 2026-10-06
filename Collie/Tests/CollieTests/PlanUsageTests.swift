import CollieCore
import Foundation
import Testing

@testable import Collie

private let now = Date(timeIntervalSince1970: 2_000_000_000)
private let nowMs = UInt64(2_000_000_000_000)

@Test func planUsageShowsWhatIsLeftAndWhenItResets() {
    let usage = PlanUsage(
        fiveHour: UsageWindow(usedPercent: 24, resetsAtMs: nowMs + 7_980_000),
        sevenDay: UsageWindow(usedPercent: 41, resetsAtMs: nowMs + 277_200_000),
        recordedMs: nowMs - 60_000
    )
    #expect(
        usage.limits(now: now) == [
            UsageLimit(label: "5h", left: 76, resetsIn: "2h 13m"),
            UsageLimit(label: "7d", left: 59, resetsIn: "3d 5h"),
        ]
    )
    #expect(!usage.isStale(now: now))
    #expect(usage.isStale(now: now.addingTimeInterval(5 * 60)))
}

@Test func planUsageLeavesOutAWindowThatHasReset() {
    let usage = PlanUsage(
        fiveHour: UsageWindow(usedPercent: 100, resetsAtMs: nowMs - 1),
        sevenDay: UsageWindow(usedPercent: 100, resetsAtMs: nowMs + 30_000),
        recordedMs: nowMs
    )
    #expect(usage.limits(now: now) == [UsageLimit(label: "7d", left: 0, resetsIn: "<1m")])
    #expect(PlanUsage(fiveHour: nil, sevenDay: nil, recordedMs: nowMs).limits(now: now).isEmpty)
    #expect(PlanUsage.countdown(seconds: 45 * 60) == "45m")
}
