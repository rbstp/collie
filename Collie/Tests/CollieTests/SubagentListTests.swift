import CollieCore
import GhosttyTerminal
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

    func testOneLinePerSubagent() {
        let long = String(repeating: "Checking the wake sentence in architecture.md ", count: 6)
        let height = { (subagents: [SubagentSummary]) in
            UIHostingController(rootView: SubagentList(subagents: subagents))
                .sizeThatFits(in: CGSize(width: 402, height: CGFloat.greatestFiniteMagnitude)).height
        }
        let typical = [subagent("#300 landscape on iPhone", tokens: 270_800)]
        let one = height(typical)
        XCTAssertGreaterThan(one, 0)
        let ideal = UIHostingController(rootView: SubagentList(subagents: typical).fixedSize())
            .sizeThatFits(in: CGSize(width: CGFloat.greatestFiniteMagnitude, height: CGFloat.greatestFiniteMagnitude))
        XCTAssert((100...370).contains(ideal.width), "a typical title and its usage fit a portrait iPhone without truncating: \(ideal.width)")
        XCTAssertEqual(height([subagent(long, tokens: 500)]), one, accuracy: 0.5)
        XCTAssertEqual(height([subagent(long, tokens: 500), subagent("Running tests", tokens: 1_200)]), one * 2 + 2, accuracy: 1)
    }

    func testLongTitlesTruncateAndLargeTextMovesTheUsageBelow() {
        let row = subagent(String(repeating: "Review the pairing flow ", count: 8), tokens: 496_517)
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

    func testGridPreviewKeepsLatestOutputVisibleAtAccessibilitySizes() async throws {
        let output = (1...100).map { "Output line \($0)\r\n" }.joined()
        let core = DemoAgentCore(snapshot: output, subagentDemo: true)
        let sample = try XCTUnwrap(core.agentView(machineId: DemoAgentCore.machine.id, terminalId: "term_demo", afterRevision: 0)?.agent)
        for kind in ["claude", "copilot", "codex"] {
            var agent = sample
            agent.kind = kind
            for starred in [false, true] {
                for size in [DynamicTypeSize.large, .accessibility5] {
                    func card(_ agent: AgentSummary) -> some View {
                        AgentCard(agent: agent, screen: output, workspace: nil, followed: false, starred: starred, now: .now)
                            .environment(\.dynamicTypeSize, size)
                            .frame(width: 370)
                    }
                    let host = UIHostingController(rootView: card(agent))
                    try await hosting(host) { window in
                        try await Task.sleep(for: .milliseconds(250))
                        host.view.layoutIfNeeded()
                        let terminal = try XCTUnwrap(descendants(host.view).compactMap { $0 as? GhosttyTerminalUIView }.first)
                        let lineHeight = UIHostingController(rootView: Text("x").font(.caption2).environment(\.dynamicTypeSize, size))
                            .sizeThatFits(in: CGSize(width: 370, height: CGFloat.greatestFiniteMagnitude)).height
                        let listHeight = lineHeight * (starred ? 5 : 3) + (starred ? 8 : 4) + 12
                        XCTAssertGreaterThan(terminal.bounds.height - listHeight, 40)
                        XCTAssertEqual(terminal.contentOffset.y, max(terminal.contentSize.height + terminal.adjustedContentInset.bottom - terminal.bounds.height, -terminal.adjustedContentInset.top), accuracy: 1)
                        let attachment = XCTAttachment(image: UIGraphicsImageRenderer(bounds: window.bounds).image { _ in
                            window.drawHierarchy(in: window.bounds, afterScreenUpdates: true)
                        })
                        attachment.name = "grid-subagents-\(kind)-\(starred)-\(size)"
                        attachment.lifetime = .keepAlways
                        add(attachment)

                        for status in [AgentState.blocked, .working] {
                            var updated = agent
                            updated.status = status
                            if status == .working { updated.subagents = [] }
                            host.rootView = card(updated)
                            try await Task.sleep(for: .milliseconds(100))
                            host.view.layoutIfNeeded()
                            XCTAssertEqual(terminal.bounds.height, starred ? 240 : 120, accuracy: 1)
                            XCTAssertEqual(terminal.contentOffset.y, max(terminal.contentSize.height + terminal.adjustedContentInset.bottom - terminal.bounds.height, -terminal.adjustedContentInset.top), accuracy: 1)
                        }
                    }
                }
            }
        }
    }
}
