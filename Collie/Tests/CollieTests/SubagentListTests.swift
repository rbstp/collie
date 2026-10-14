import CollieCore
import SwiftUI
import XCTest
@testable import Collie

@MainActor
final class SubagentListTests: XCTestCase {
    private let started: UInt64 = 1_700_000_000_000

    private func subagent(_ activity: String, tokens: UInt64, calledTools: Bool = true) -> SubagentSummary {
        SubagentSummary(agentType: "general-purpose", activity: activity, startedAtMs: started, tokens: tokens, calledTools: calledTools)
    }

    private func at(_ seconds: Double) -> Date { Date(timeIntervalSince1970: Double(started / 1000) + seconds) }

    func testRowsReadLikeTheTerminalPanel() {
        XCTAssertEqual(subagent("a", tokens: 496_517).usage(now: at(263)), "4m 23s · ↓ 496.5k tokens")
        XCTAssertEqual(subagent("a", tokens: 193_412).usage(now: at(2271)), "37m 51s · ↓ 193.4k tokens")
        XCTAssertEqual(subagent("a", tokens: 500, calledTools: false).usage(now: at(6)), "6s · ↑ 500 tokens")
        XCTAssertEqual(subagent("a", tokens: 0, calledTools: false).usage(now: at(-5)), "0s")
        XCTAssertEqual(subagent("a", tokens: 2_049).usage(now: at(3_725)), "1h 2m 5s · ↓ 2k tokens")
        XCTAssertEqual(SubagentSummary.count(999_960), "1m")
        XCTAssertEqual(SubagentSummary.count(1_250_000), "1.3m")
        XCTAssertEqual(SubagentSummary.count(999), "999")
        XCTAssertEqual(SubagentSummary.count(400), "400")
    }

    func testOneLinePerSubagent() {
        let long = String(repeating: "Checking the wake sentence in architecture.md ", count: 6)
        let height = { (subagents: [SubagentSummary]) in
            UIHostingController(rootView: SubagentList(subagents: subagents))
                .sizeThatFits(in: CGSize(width: 402, height: CGFloat.greatestFiniteMagnitude)).height
        }
        let one = height([subagent("Reading lib.rs", tokens: 500)])
        XCTAssertGreaterThan(one, 0)
        XCTAssertEqual(height([subagent(long, tokens: 500)]), one, accuracy: 0.5)
        XCTAssertEqual(height([subagent(long, tokens: 500), subagent("Running tests", tokens: 1_200)]), one * 2 + 2, accuracy: 1)
    }

    func testLongTypesTruncateAndLargeTextMovesTheUsageBelow() {
        let type = String(repeating: "very-long-custom-agent-type-", count: 8)
        let row = SubagentSummary(agentType: type, activity: "Reading pairing.rs", startedAtMs: started, tokens: 496_517, calledTools: true)
        let fit = { (view: AnyView) in
            UIHostingController(rootView: view).sizeThatFits(in: CGSize(width: 370, height: CGFloat.greatestFiniteMagnitude))
        }
        for size in [DynamicTypeSize.large, .accessibility5] {
            let list = fit(AnyView(SubagentList(subagents: [row]).environment(\.dynamicTypeSize, size)))
            let line = fit(AnyView(Text(verbatim: "x").font(.caption).environment(\.dynamicTypeSize, size))).height
            XCTAssertLessThanOrEqual(list.width, 370.5, "\(size)")
            if size.isAccessibilitySize {
                XCTAssertGreaterThan(list.height, line * 1.5, "\(size)")
            } else {
                XCTAssertLessThan(list.height, line * 1.5, "\(size)")
            }
        }
    }
}
