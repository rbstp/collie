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
    let faster = usage(fiveHour: 66, resetsIn: 7_200, sevenDay: 90, resetsIn: 302_400).limits(now: now)
    #expect(faster.map(\.pace) == [.faster, .faster])
    let onPace = usage(fiveHour: 57, resetsIn: 7_200, sevenDay: 47, resetsIn: 302_400).limits(now: now)
    #expect(onPace.map(\.pace) == [.onPace, .onPace])
    #expect(onPace[1].spokenLine(now: now).contains("50 percent of the window elapsed, usage on pace, resets in"))
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
    #expect(limits.map(\.title) == ["5-hour usage limit", "Weekly usage limit"])
    #expect(limits[1].resetsLine(now: now) == "resets in 3d 16h")
    #expect(limits[0].resetsLine(now: now).hasPrefix("resets at "))
    #expect(limits[1].spokenLine(now: now) == "Weekly usage limit, 22 percent used, 48 percent of the window elapsed, usage pace slower, resets in 3 days, 16 hours")
    #expect(limits[0].spokenLine(now: now).hasPrefix("5-hour usage limit, 75 percent used, 56 percent of the window elapsed, usage pace faster, resets at "))
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

@Test func codexPaceUsesTheElapsedCalendarMonth() throws {
    for (start, reset) in [
        ("2026-10-01T00:00:00Z", "2026-11-01T00:00:00Z"),
        ("2026-02-01T00:00:00Z", "2026-03-01T00:00:00Z"),
        ("2028-02-01T00:00:00Z", "2028-03-01T00:00:00Z"),
        ("2026-12-01T00:00:00Z", "2027-01-01T00:00:00Z"),
    ] {
        let beginning = try #require(ISO8601DateFormatter().date(from: start))
        let end = try #require(ISO8601DateFormatter().date(from: reset))
        let usage = CodexUsage(used: 50, limit: 100, resetsAtMs: end.unixMs, recordedMs: beginning.unixMs)
        #expect(usage.elapsed(now: beginning.addingTimeInterval(-1)) == 0)
        #expect(usage.elapsed(now: beginning) == 0)
        #expect(usage.elapsed(now: beginning.addingTimeInterval(end.timeIntervalSince(beginning) / 2)) == 0.5)
        #expect(usage.elapsed(now: end) == 1)
        #expect(usage.elapsed(now: end.addingTimeInterval(1)) == 1)
    }
}

@Test func newTaskWarnsAboutTheFullestWindowOfTheAgentsPlan() throws {
    let now = Date.now
    let demo = try #require(DemoAgentCore().cachedFlock(machineId: DemoAgentCore.machine.id)?.planUsage)
    for agent in ["claude", "codex", "copilot"] {
        #expect(demo.warning(agent: agent, now: now) == nil, "the demo's 38% and 61%, and no Codex reading")
    }
    let resets = { (usage: PlanUsage, label: String) in usage.limits(now: now).first { $0.label == label }?.resetsLine(now: now) ?? "" }

    var usage = demo
    usage.fiveHour?.usedPercent = 92
    var warning = try #require(usage.warning(agent: "claude", now: now))
    #expect(warning.text == "Claude 5-hour limit at 92%, \(resets(usage, "5h"))")
    #expect(warning.text.hasPrefix("Claude 5-hour limit at 92%, resets at "))
    #expect(warning.spoken == warning.text)
    #expect(!warning.reached)
    #expect(usage.warning(agent: "copilot", now: now) == nil)
    #expect(usage.warning(agent: "codex", now: now) == nil)

    usage.fiveHour?.usedPercent = 100
    warning = try #require(usage.warning(agent: "claude", now: now))
    #expect(warning.text == "Claude 5-hour limit reached, \(resets(usage, "5h"))")
    #expect(warning.reached)

    usage.fiveHour?.usedPercent = 79
    #expect(usage.warning(agent: "claude", now: now) == nil)
    usage.sevenDay?.usedPercent = 85
    warning = try #require(usage.warning(agent: "claude", now: now))
    #expect(warning.text == "Claude weekly limit at 85%, \(resets(usage, "7d"))")
    #expect(warning.text.hasPrefix("Claude weekly limit at 85%, resets in 3d "))
    #expect(warning.spoken.hasPrefix("Claude weekly limit at 85%, resets in 3 days, "))

    usage.fiveHour?.usedPercent = 100
    usage.sevenDay?.usedPercent = 100
    #expect(usage.warning(agent: "claude", now: now)?.text.hasPrefix("Claude weekly limit reached") == true, "the later reset")

    usage.codex = CodexUsage(used: 2_000, limit: 2_000, resetsAtMs: now.unixMs + 9 * 86_400_000, recordedMs: now.unixMs - 20_000)
    warning = try #require(usage.warning(agent: "codex", now: now))
    #expect(warning.text == "Codex monthly limit reached, resets \(Date(unixMs: now.unixMs + 9 * 86_400_000).formatted(date: .abbreviated, time: .omitted))")
    #expect(warning.reached)
    #expect(usage.warning(agent: "copilot", now: now) == nil)
    usage.codex?.used = 1_600
    #expect(usage.warning(agent: "codex", now: now)?.text.hasPrefix("Codex monthly limit at 80%, resets ") == true)
    usage.codex?.used = 1_580
    #expect(usage.warning(agent: "codex", now: now) == nil)
    usage.codex?.used = 2_000
    usage.codex?.resetsAtMs = now.unixMs - 1
    #expect(usage.warning(agent: "codex", now: now) == nil, "the Usage view hides a reading past its reset")
}

@Test func newTaskWarningSaysHowOldAStaleReadingIs() throws {
    var usage = PlanUsage(
        fiveHour: UsageWindow(usedPercent: 95, resetsAtMs: nowMs + 3_600_000), sevenDay: nil, recordedMs: nowMs - 4 * 60_000,
        codex: CodexUsage(used: 95, limit: 100, resetsAtMs: nowMs + 86_400_000, recordedMs: nowMs - 9 * 60_000)
    )
    #expect(usage.warning(agent: "claude", now: now)?.text.contains("recorded") == false)
    #expect(usage.warning(agent: "codex", now: now)?.text.contains("recorded") == false)
    usage.recordedMs = nowMs - 12 * 60_000
    usage.codex?.recordedMs = nowMs - 11 * 60_000
    let claude = try #require(usage.warning(agent: "claude", now: now))
    #expect(claude.text.hasPrefix("Claude 5-hour limit at 95%, resets at "))
    #expect(claude.text.hasSuffix(", recorded 12m ago"))
    #expect(claude.spoken.hasSuffix(", recorded 12 minutes ago"))
    #expect(usage.warning(agent: "codex", now: now)?.text.hasSuffix(", recorded 11m ago") == true)
}

@Test func copilotPremiumUsageWarnsLikeCodexAndCountsOverage() throws {
    var usage = PlanUsage(
        fiveHour: nil, sevenDay: nil, recordedMs: 0,
        copilot: CopilotUsage(used: 239_413, limit: 400_000, unlimited: false, tokenBasedBilling: true, resetsAtMs: nowMs + 86_400_000, recordedMs: nowMs - 20_000)
    )
    #expect(usage.copilot?.percent == 60)
    #expect(usage.copilot?.unit == "credits")
    #expect(usage.warning(agent: "copilot", now: now) == nil)
    usage.copilot?.used = 320_000
    #expect(usage.warning(agent: "copilot", now: now)?.text.hasPrefix("Copilot premium limit at 80%, resets ") == true)
    usage.copilot?.used = 412_500
    #expect(usage.copilot?.percent == 103)
    let warning = try #require(usage.warning(agent: "copilot", now: now))
    #expect(warning.text.hasPrefix("Copilot premium limit reached, resets "))
    #expect(warning.reached)
    usage.copilot?.recordedMs = nowMs - 11 * 60_000
    #expect(usage.warning(agent: "copilot", now: now)?.text.hasSuffix(", recorded 11m ago") == true)
    #expect(usage.age(now: now) == "11m ago")
    usage.copilot?.tokenBasedBilling = false
    #expect(usage.copilot?.unit == "requests")
    usage.copilot?.unlimited = true
    #expect(usage.warning(agent: "copilot", now: now) == nil)
    usage.copilot?.unlimited = false
    usage.copilot?.resetsAtMs = nowMs - 1
    #expect(usage.warning(agent: "copilot", now: now) == nil)
}
