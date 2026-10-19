import CollieCore
import Foundation
import SwiftUI
import Testing
import UIKit

@testable import Collie

private func costs(scanning: Bool, cost: Double = 1.5) -> UsageCosts {
    UsageCosts(
        scanning: scanning,
        tokens: CostTokens(input: 1_200, output: 300, cacheRead: 50_000, cacheWrite: 2_000),
        costUsd: cost,
        cacheSavingsUsd: 0.2,
        days: [DayCost(date: "2026-10-17", costUsd: 0.5), DayCost(date: "2026-10-18", costUsd: cost - 0.5)],
        models: [
            ModelCost(agent: "claude", model: "claude-opus-5-5", tokens: CostTokens(input: 1_000, output: 200, cacheRead: 50_000, cacheWrite: 2_000), costUsd: cost, cacheSavingsUsd: 0.2),
            ModelCost(agent: "copilot", model: "unknown", tokens: CostTokens(input: 200, output: 100, cacheRead: 0, cacheWrite: 0), costUsd: nil, cacheSavingsUsd: nil),
        ]
    )
}

@MainActor @Test func usageCostsReadAgainWhileCollieIsScanning() async {
    let model = UsageCostsModel()
    var calls: [(CostRange, Int16)] = []
    await model.load(range: .week, offsetMinutes: -240, every: .zero) { range, offset in
        calls.append((range, offset))
        return costs(scanning: calls.count < 3, cost: Double(calls.count))
    }
    #expect(calls.count == 3)
    #expect(calls.allSatisfy { $0.0 == .week && $0.1 == -240 })
    #expect(model.phase == .loaded(costs(scanning: false, cost: 3)))
}

@MainActor @Test func usageCostsAskForANewerCollied() async {
    let model = UsageCostsModel()
    await model.load(range: .today) { _, _ in throw CoreError.NotImplemented }
    #expect(model.phase == .outdated)
}

@MainActor @Test func usageCostsKeepTheLastFiguresWhenARefreshFails() async {
    let model = UsageCostsModel()
    await model.load(range: .today) { _, _ in throw CoreError.HerdrUnavailable }
    #expect(model.phase == .failed("herdr is not running on the machine"))
    await model.load(range: .today) { _, _ in costs(scanning: false) }
    await model.load(range: .today) { _, _ in throw CoreError.RateLimited }
    #expect(model.phase == .loaded(costs(scanning: false)))
}

@Test func usageCostRowsNameTheAgentAndAModelWithoutAPrice() {
    let rows = costs(scanning: false).models
    #expect(rows.map(\.agentName) == ["Claude Code", "Copilot CLI"])
    #expect(rows[1].costText == "No price")
    #expect(rows[0].costText != "No price")
    #expect(rows[0].tokens.total == 53_200)
    #expect(DayCost(date: "2026-10-18", costUsd: 0).day == Calendar.current.date(from: DateComponents(year: 2026, month: 10, day: 18)))
    #expect(DayCost(date: "October", costUsd: 0).day == nil)
    #expect([CostRange.today, .week, .month].map(\.title) == ["Today", "7 days", "30 days"])
}

@MainActor @Test func usageCostsCardReadsTheChosenRange() async throws {
    let core = FakeCore()
    core.state.withLock { $0.costs = costs(scanning: false) }
    let host = UIHostingController(rootView: List { UsageCostsCard(core: core, machineId: "m1") })
    try await hosting(host) { _ in
        let picker = try await waitFor("range picker") {
            host.view.layoutIfNeeded()
            return descendants(host.view).compactMap { $0 as? UISegmentedControl }.first { $0.titleForSegment(at: 0) == "Today" }
        }
        await poll { core.snapshot.costRanges == [.week] }
        #expect(picker.selectedSegmentIndex == 1)
        picker.selectedSegmentIndex = 2
        picker.sendActions(for: .valueChanged)
        await poll { core.snapshot.costRanges == [.week, .month] }
    }
}
