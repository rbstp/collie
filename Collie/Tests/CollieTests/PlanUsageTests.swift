import CollieCore
import Foundation
import Testing

@testable import Collie

private let now = Date(timeIntervalSince1970: 2_000_000_000)
private let nowMs = UInt64(2_000_000_000_000)

private func usage(fiveHour: UInt8, resetsIn fiveHourSeconds: UInt64, sevenDay: UInt8, resetsIn sevenDaySeconds: UInt64) -> PlanUsage {
    PlanUsage(
        fiveHour: UsageWindow(usedPercent: fiveHour, resetsAtMs: nowMs + fiveHourSeconds * 1000),
        sevenDay: UsageWindow(usedPercent: sevenDay, resetsAtMs: nowMs + sevenDaySeconds * 1000),
        recordedMs: nowMs - 60_000
    )
}

@Test func planUsageElapsedShareComesFromTheResetTimeAndTheWindow() {
    let limits = usage(fiveHour: 24, resetsIn: 7_200, sevenDay: 41, resetsIn: 302_400).limits(now: now)
    #expect(limits.map(\.label) == ["5h", "7d"])
    #expect(limits.map(\.used) == [24, 41])
    #expect(limits.map(\.left) == [76, 59])
    #expect(limits.map(\.length) == [18_000, 604_800])
    #expect(limits.map(\.elapsed) == [0.6, 0.5])
    let fresh = usage(fiveHour: 0, resetsIn: 7_200, sevenDay: 0, resetsIn: 1)
    #expect(!fresh.isStale(now: now))
    #expect(fresh.isStale(now: now.addingTimeInterval(5 * 60)))
    #expect(!PlanUsage(fiveHour: nil, sevenDay: nil, recordedMs: .max).isStale(now: now))
}

@Test func planUsagePaceComparesUsedWithElapsed() {
    // 5h: 60% of the window has passed. 7d: 50%.
    let slower = usage(fiveHour: 54, resetsIn: 7_200, sevenDay: 20, resetsIn: 302_400).limits(now: now)
    #expect(slower.map(\.pace) == [.slower, .slower])
    #expect(slower.map(\.paceText) == ["5h usage pace slower", "7d usage pace slower"])
    #expect(slower.map(\.spokenPace) == ["5-hour usage pace slower", "Weekly usage pace slower"])
    let faster = usage(fiveHour: 66, resetsIn: 7_200, sevenDay: 90, resetsIn: 302_400).limits(now: now)
    #expect(faster.map(\.pace) == [.faster, .faster])
    #expect(faster.map(\.paceText) == ["5h usage pace faster", "7d usage pace faster"])
    let onPace = usage(fiveHour: 57, resetsIn: 7_200, sevenDay: 47, resetsIn: 302_400).limits(now: now)
    #expect(onPace.map(\.pace) == [.onPace, .onPace])
    #expect(onPace.map(\.paceText) == ["5h usage on pace", "7d usage on pace"])
}

@Test func planUsageLeavesOutAWindowPastItsReset() {
    let usage = PlanUsage(
        fiveHour: UsageWindow(usedPercent: 100, resetsAtMs: nowMs - 1),
        sevenDay: UsageWindow(usedPercent: 100, resetsAtMs: nowMs + 30_000),
        recordedMs: nowMs
    )
    let limits = usage.limits(now: now)
    #expect(limits == [UsageLimit(label: "7d", used: 100, seconds: 30, length: 604_800)])
    #expect(limits.first.map { $0.elapsed > 0.99 } == true)
    #expect(limits.first?.pace == .onPace)
    #expect(limits.first?.resets(now: now) == "<1m")
    #expect(PlanUsage(fiveHour: nil, sevenDay: nil, recordedMs: nowMs).limits(now: now).isEmpty)
    // A reset further away than the window's length (clock skew) counts as just started.
    let early = UsageLimit(label: "5h", used: 0, seconds: 20_000, length: 18_000)
    #expect(early.elapsed == 0 && early.pace == .onPace)
}

@Test func planUsageFormatsResetsAndAge() {
    let style = Date.FormatStyle(date: .omitted, time: .shortened, locale: Locale(identifier: "en_GB"), timeZone: TimeZone(secondsFromGMT: 11 * 3600)!)
    let limits = usage(fiveHour: 75, resetsIn: 7_980, sevenDay: 22, resetsIn: 317_000).limits(now: now)
    // 2_000_000_000 is 14:33:20 at UTC+11; 2h 13m later is 16:46.
    #expect(limits[0].resets(now: now, style: style) == "16:46")
    #expect(limits[1].resets(now: now, style: style) == "3d 16h")
    #expect(limits[1].spokenLine(now: now) == "Weekly limit, 22 percent used, 48 percent of the window elapsed, resets in 3 days, 16 hours")
    #expect(limits[0].spokenLine(now: now).hasPrefix("5-hour limit, 75 percent used, 56 percent of the window elapsed, resets at "))
    #expect(PlanUsage.countdown(seconds: 45 * 60) == "45m")
    let recorded = { (ago: UInt64) in PlanUsage(fiveHour: nil, sevenDay: nil, recordedMs: nowMs - ago * 1000) }
    #expect(recorded(20).age(now: now) == "just now")
    #expect(recorded(20).spokenAge(now: now) == "recorded just now")
    #expect(recorded(180).age(now: now) == "3m ago")
    #expect(recorded(90_000).age(now: now) == "1d ago")
    #expect(recorded(7_980).spokenAge(now: now) == "recorded 2 hours, 13 minutes ago")
    #expect(PlanUsage(fiveHour: nil, sevenDay: nil, recordedMs: nowMs + 5_000).age(now: now) == "just now")
}

@Test func planUsageSpellsOutDurationsForVoiceOver() {
    #expect(PlanUsage.spoken(seconds: 7_980) == "2 hours, 13 minutes")
    #expect(PlanUsage.spoken(seconds: 277_200) == "3 days, 5 hours")
    #expect(PlanUsage.spoken(seconds: 30) == "1 minute")
}
