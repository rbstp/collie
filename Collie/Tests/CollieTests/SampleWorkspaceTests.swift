import CollieCore
import Testing

@testable import Collie

@Test func sampleDecisionResolvesOnlyLocalState() async throws {
    let core = DemoAgentCore()
    let original = DemoAgentCore()
    let machine = DemoAgentCore.machine.id
    let approval = try #require(core.approvalFeed(machineId: machine)?.pending.first)
    let outcome = try await core.decide(machineId: machine, approvalId: approval.approvalId, decision: .deny, note: nil)
    #expect(outcome == .applied(decision: .deny, by: "sample"))
    #expect(core.approvalFeed(machineId: machine)?.pending.isEmpty == true)
    #expect(core.cachedFlock(machineId: machine)?.agents.first { $0.terminalId == approval.terminalId }?.status == .done)
    #expect(original.approvalFeed(machineId: machine)?.pending.count == 1)
    #expect(try await core.decide(machineId: machine, approvalId: approval.approvalId, decision: .approve, note: nil) == .expired)
}

@Test func samplePromptsAndTerminalKeysProduceVisibleLocalResults() async throws {
    let core = DemoAgentCore()
    let machine = DemoAgentCore.machine.id
    let terminal = "term_demo"
    let initial = try #require(core.agentView(machineId: machine, terminalId: terminal, afterRevision: 0))
    let history = try await core.agentHistory(machineId: machine, terminalId: terminal, before: nil, after: nil, query: nil)
    try await core.prompt(machineId: machine, terminalId: terminal, text: "Show sample result", expectedDraft: nil)
    try await core.sendKeys(machineId: machine, terminalId: terminal, keys: [.esc])
    let view = try #require(core.agentView(machineId: machine, terminalId: terminal, afterRevision: initial.outputRevision))
    #expect(view.outputRevision > initial.outputRevision)
    #expect(view.output?.ansi.contains("Show sample result") == true)
    #expect(view.output?.ansi.contains("Simulated terminal keys received") == true)
    let updated = try await core.agentHistory(machineId: machine, terminalId: terminal, before: nil, after: history.after, query: nil)
    #expect(updated.messages.count == 2)
    #expect(updated.messages.allSatisfy { $0.id.hasPrefix("sample-") })
    #expect(DemoAgentCore().agentView(machineId: machine, terminalId: terminal, afterRevision: 0)?.outputRevision == 1)
}
