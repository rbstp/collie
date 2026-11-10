import CollieCore
import SwiftUI
import XCTest
@testable import Collie

@MainActor
final class SubagentListTests: XCTestCase {
    private let started: UInt64 = 1_700_000_000_000

    private func subagent(_ title: String, tokens: UInt64, calledTools: Bool = true) -> SubagentSummary {
        SubagentSummary(agentType: "general-purpose", title: title, activity: "Running cargo test", startedAtMs: started, tokens: tokens, calledTools: calledTools)
    }
    private func at(_ seconds: Double) -> Date { Date(timeIntervalSince1970: Double(started / 1000) + seconds) }

    func testRowsReadLikeTheTerminalPanel() {
        XCTAssertEqual(subagent("a", tokens: 496_517).usage(now: at(263)), "4m · ↓ 496.5k")
        XCTAssertEqual(subagent("a", tokens: 193_412).usage(now: at(4_309)), "1h 11m · ↓ 193.4k")
        XCTAssertEqual(subagent("a", tokens: 500, calledTools: false).usage(now: at(59)), "<1m · ↑ 500")
        XCTAssertEqual(subagent("a", tokens: 0, calledTools: false).usage(now: at(-5)), "<1m")
        XCTAssertEqual(subagent("a", tokens: 2_049).usage(now: at(3_600)), "1h 0m · ↓ 2k")
        XCTAssertEqual(SubagentSummary.count(999_960), "1m")
        XCTAssertEqual(SubagentSummary.count(1_250_000), "1.3m")
        XCTAssertEqual(SubagentSummary.count(999), "999")
        XCTAssertEqual(SubagentSummary.count(400), "400")
    }

    func testLongTitlesTruncateAndLargeTextMovesTheUsageBelow() {
        let row = subagent(String(repeating: "Review the pairing flow ", count: 8), tokens: 496_517)
        let fit = { (view: AnyView) in
            UIHostingController(rootView: view).sizeThatFits(in: CGSize(width: 370, height: CGFloat.greatestFiniteMagnitude))
        }
        for size in [DynamicTypeSize.large, .accessibility3] {
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
