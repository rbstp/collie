import CollieCore
import Foundation
import Testing

@testable import Collie

private let noon = Date(timeIntervalSince1970: 1_791_028_800)

private func limit(_ window: LimitWindow = .fiveHour, resetsIn: TimeInterval = 3_600, resumes: Bool = true) -> PlanLimitSummary {
    let resets = Date(unixMs: noon.unixMs).addingTimeInterval(resetsIn).unixMs
    return PlanLimitSummary(window: window, resetsAtMs: resets, resumeAtMs: resumes ? resets + 60_000 : nil)
}

@MainActor
@Test func theBadgeNamesTheResumeTimeOrTheLimitAlone() {
    let armed = limit()
    let at = LimitBadge.time(armed.resumeAtMs!, now: noon)
    #expect(LimitBadge.text(armed, compact: true, now: noon) == at)
    #expect(LimitBadge.text(armed, compact: false, now: noon) == "Resumes \(at)")
    #expect(LimitBadge.spoken(armed, now: noon) == "Limited at the 5-hour limit, resumes at \(at)")
    let cancelled = limit(resumes: false)
    #expect(LimitBadge.text(cancelled, compact: true, now: noon) == nil)
    #expect(LimitBadge.text(cancelled, compact: false, now: noon) == "Limited")
    #expect(LimitBadge.spoken(cancelled, now: noon).hasSuffix("will not resume"))
}

@MainActor
@Test func aResetOnAnotherDayNamesTheDay() {
    let weekly = limit(.sevenDay, resetsIn: 3 * 86_400)
    let at = Date(unixMs: weekly.resumeAtMs!)
    #expect(LimitBadge.time(weekly.resumeAtMs!, now: noon) == at.formatted(.dateTime.weekday(.abbreviated).hour().minute()))
    #expect(LimitBadge.spoken(weekly, now: noon).hasPrefix("Limited at the weekly limit"))
}

@MainActor
@Test func usageCountsTheAgentsWaitingOnEachReset() {
    let agents = [limit(), limit(), limit(.sevenDay), nil].enumerated().map { index, limit in
        AgentSummary(
            terminalId: "t\(index)", workspaceId: "w1", kind: "claude", name: nil, title: nil, status: .idle,
            statusSinceMs: 0, cwd: nil, lastLine: nil, planLimit: limit
        )
    }
    #expect(UsageSections.waiting(agents) == ["5h": 2, "7d": 1])
}

@MainActor
@Test func cancelAndContinueNowReachTheMachine() async {
    let core = FakeCore()
    core.state.withLock { $0.kind = "claude" }
    let model = agentModel(core)
    model.poll()
    await model.cancelResume()
    #expect(core.snapshot.limitCalls == ["cancel m1 term_1"])
    await model.continueNow()
    #expect(core.snapshot.prompts == [AgentModel.continuePrompt])
    #expect(core.snapshot.expectedDrafts == [""], "never replaces unsent text in the box")
    core.set(error: .Rejected(message: "no resume is armed for the agent"))
    await model.cancelResume()
    #expect(model.notice != nil)
}
