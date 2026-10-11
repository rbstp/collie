import CollieCore
import Foundation
import SwiftUI
import Testing

@testable import Collie

private func message(_ id: String, _ text: String = "hello") -> HistoryMessage {
    HistoryMessage(id: id, role: "assistant", text: text, tool: nil, callId: nil, truncated: false)
}

private func page(_ messages: [HistoryMessage], session: String = "s1", reset: Bool = false, before: String? = "older", after: String = "latest") -> HistoryPage {
    HistoryPage(session: session, available: true, unavailableReason: nil, reset: reset, messages: messages, before: before, after: after, more: false, omitted: false)
}

@MainActor
@Test func copilotConversationLoadsThroughTheAgentAndRetainsReconnectCursor() async {
    let core = FakeCore()
    let session = "copilot:11111111-2222-4333-8444-555555555555:1:2"
    core.state.withLock {
        $0.kind = "copilot"
        $0.historyPage = page([
            HistoryMessage(id: "u1:0", role: "user", text: "Fix the café counter.", tool: nil, callId: nil, truncated: false),
            HistoryMessage(id: "t1:0", role: "assistant", text: "{\"path\":\"counter.rs\"}", tool: "view", callId: "read-1", truncated: false),
            HistoryMessage(id: "t2:0", role: "tool", text: "let count = 2;", tool: "Result", callId: "read-1", truncated: false),
            message("a1:0", "**Fixed** the café counter.")
        ], session: session)
    }
    let model = AgentModel(core: core, route: AgentRoute(machineId: "m1", terminalId: "t1"), prefsFile: nil)
    model.poll()
    #expect(model.agent?.kind == "copilot")
    #expect(!model.isTerminal)
    await model.conversation.fetch(load: model.history)
    #expect(model.conversation.available == true)
    #expect(model.conversation.rows.count == 3)
    #expect(core.snapshot.historyRequests == [.init(machine: "m1", terminal: "t1", before: nil, after: nil, query: nil)])
    model.conversation.stop()
    model.conversation.begin(query: "")
    core.state.withLock { $0.historyPage = page([message("a2:0", "Verified.")], session: session, after: "next") }
    await model.conversation.fetch(load: model.history)
    #expect(core.snapshot.historyRequests.last?.after == "latest")
    #expect(model.conversation.messages.map(\.id) == ["u1:0", "t1:0", "t2:0", "a1:0", "a2:0"])
    model.conversation.begin(query: "Verified")
    await model.conversation.fetch(load: model.history)
    #expect(core.snapshot.historyRequests.last?.query == "Verified")
    #expect(core.snapshot.historyRequests.last?.after == nil)
}

@MainActor
@Test func copilotConversationKeepsTerminalFallbackForOlderDaemons() async {
    let core = FakeCore()
    core.state.withLock { $0.kind = "copilot" }
    let model = AgentModel(core: core, route: AgentRoute(machineId: "m1", terminalId: "t1"), prefsFile: nil)
    model.poll()
    await model.conversation.fetch(load: model.history)
    #expect(model.conversation.available == false)
    #expect(model.agent?.kind == "copilot")
    #expect(model.conversation.messages.isEmpty)
}

@MainActor
@Test func conversationMergesWithoutDuplicateMessagesAndKeepsBothCursors() {
    let model = ConversationModel()
    model.apply(page([message("2"), message("3")]), older: false)
    model.apply(page([message("1"), message("2")], before: nil, after: "oldEnd"), older: true)
    #expect(model.messages.map(\.id) == ["1", "2", "3"])
    #expect(model.before == nil)
    #expect(model.after == "latest")
    model.apply(page([message("3", "updated"), message("4")], after: "newEnd"), older: false)
    #expect(model.messages.map(\.id) == ["1", "2", "3", "4"])
    #expect(model.messages[2].text == "updated")
    #expect(model.after == "newEnd")
    #expect(model.before == nil)
}

@MainActor
@Test func conversationSessionChangeAndFileResetDiscardOldMessages() {
    let model = ConversationModel()
    model.apply(page([message("old")]), older: false)
    model.apply(page([message("new")], session: "s2"), older: false)
    #expect(model.messages.map(\.id) == ["new"])
    model.apply(page([message("rewritten")], session: "s2", reset: true), older: true)
    #expect(model.messages.map(\.id) == ["rewritten"])
}

@MainActor
@Test func conversationDistinguishesUnavailableAndEmpty() {
    let model = ConversationModel()
    #expect(model.available == nil)
    model.apply(page([], before: nil), older: false)
    #expect(model.available == true)
    model.apply(HistoryPage(session: nil, available: false, unavailableReason: "Session ID missing", reset: false, messages: [], before: nil, after: nil, more: false, omitted: false), older: false)
    #expect(model.available == false)
    #expect(model.unavailableReason == "Session ID missing")
    #expect(model.messages.isEmpty)
    #expect(model.after == nil)
    model.apply(page([message("recovered")]), older: false)
    #expect(model.available == true)
    #expect(model.unavailableReason == nil)
}

@MainActor
@Test func conversationReconnectRetainsHistoryAndUsesIncrementalCursor() async {
    let model = ConversationModel()
    model.apply(page([message("1")]), older: false)
    model.begin(query: "")
    await model.fetch { before, after, query in
        #expect(before == nil)
        #expect(after == "latest")
        #expect(query == nil)
        return page([message("2")], after: "next")
    }
    #expect(model.messages.map(\.id) == ["1", "2"])
}

@MainActor
@Test func conversationSearchResetsAndDoesNotAcceptLateResponses() async {
    let model = ConversationModel()
    model.apply(page([message("old")]), older: false)
    var pending: CheckedContinuation<HistoryPage, Never>?
    let task = Task {
        await model.fetch { _, _, _ in
            await withCheckedContinuation { pending = $0 }
        }
    }
    while pending == nil { await Task.yield() }
    model.begin(query: "needle")
    await model.fetch { before, after, query in
        #expect(before == nil && after == nil)
        #expect(query == "needle")
        return page([message("match")])
    }
    pending?.resume(returning: page([message("late")]))
    await task.value
    #expect(model.messages.map(\.id) == ["match"])
    #expect(!model.loading)
}

@MainActor
@Test func conversationCancellationDoesNotApplyAnInflightPage() async {
    let model = ConversationModel()
    var pending: CheckedContinuation<HistoryPage, Never>?
    let task = Task {
        await model.fetch { _, _, _ in await withCheckedContinuation { pending = $0 } }
    }
    while pending == nil { await Task.yield() }
    task.cancel()
    pending?.resume(returning: page([message("late")]))
    await task.value
    #expect(model.messages.isEmpty)
    #expect(!model.loading)
}

@Test func conversationMarkdownSeparatesFencedCodeAndPreservesWhitespace() {
    let blocks = ConversationMarkdown.blocks("**Done**\n```swift\nlet x = 1\n  run(x)\n```\nNext")
    #expect(blocks.count == 3)
    #expect(!blocks[0].code)
    #expect(blocks[1].code)
    #expect(blocks[1].text == "let x = 1\n  run(x)")
    #expect(blocks[2].text == "Next")
}

@MainActor
@Test func conversationDeduplicatesRepeatedRecordsWithinOnePage() {
    let model = ConversationModel()
    model.apply(page([message("1", "partial"), message("1", "complete")]), older: false)
    #expect(model.messages.count == 1)
    #expect(model.messages[0].text == "complete")
}

@Test func conversationLinksOnlyOpenWebURLs() {
    for value in ["https://example.com/path", "http://localhost:8080", "HTTPS://example.com"] {
        #expect(ConversationMarkdown.allowsLink(URL(string: value)!))
    }
    for value in ["file:///etc/passwd", "collie://approve", "javascript:alert(1)", "data:text/html,hello", "https:relative", "relative/path"] {
        #expect(!ConversationMarkdown.allowsLink(URL(string: value)!))
    }
}

@MainActor
@Test func conversationSearchFitsTheWireLimitWithoutSplittingCharacters() {
    #expect(ConversationModel.boundedQuery(String(repeating: "a", count: 300)).utf8.count == 256)
    let emoji = "👩🏽‍💻"
    let query = ConversationModel.boundedQuery(String(repeating: emoji, count: 64))
    #expect(query.utf8.count <= 256)
    #expect(query.allSatisfy { String($0) == emoji })
    #expect(ConversationModel.boundedQuery("Résumé") == "Résumé")
}

private func tool(_ id: String, call: String? = nil, result: Bool = false) -> HistoryMessage {
    HistoryMessage(id: id, role: result ? "tool" : "assistant", text: "body", tool: result ? "Result" : "exec", callId: call, truncated: false)
}

@Test func conversationGroupsActivityAndPairsInterleavedResults() {
    let rows = ConversationRow.build([
        message("before"), tool("a", call: "one"), tool("b", call: "two"),
        tool("br", call: "two", result: true), tool("ar", call: "one", result: true), message("after"),
    ])
    #expect(rows.count == 3)
    #expect(rows[0].message?.id == "before")
    #expect(rows[1].tools.count == 2)
    #expect(rows[1].tools[0].records.map(\.id) == ["a", "ar"])
    #expect(rows[1].tools[1].records.map(\.id) == ["b", "br"])
    #expect(rows[2].message?.id == "after")
}

@Test func conversationActivityKeepsItsAnchorAcrossPagesAndLiveUpdates() {
    let initial = ConversationRow.build([tool("result", call: "one", result: true)])
    let prepended = ConversationRow.build([tool("call", call: "one"), tool("result", call: "one", result: true)], previous: initial)
    #expect(prepended[0].id == initial[0].id)
    #expect(prepended[0].tools.count == 1)
    #expect(prepended[0].tools[0].name == "exec")
    let appended = ConversationRow.build([
        tool("call", call: "one"), tool("result", call: "one", result: true), tool("next", call: "two"), message("reply"),
    ], previous: prepended)
    #expect(appended[0].id == initial[0].id)
    #expect(appended[0].tools.count == 2)
}

@Test func conversationKeepsUnpairedToolsAndEverySearchMatchVisible() {
    let records = [tool("orphan", call: "missing", result: true), tool("anonymous"), tool("another"), message("reply")]
    let rows = ConversationRow.build(records)
    #expect(rows[0].tools.count == 3)
    #expect(rows[0].tools[0].name == "Tool result")
    #expect(Set(rows.flatMap(\.recordIDs)) == Set(records.map(\.id)))
    let matches = ConversationRow.build(records, previous: rows, searching: true)
    #expect(matches.count == records.count)
    #expect(matches.allSatisfy { $0.message != nil && $0.tools.isEmpty })
    #expect(matches.map(\.id) == records.map { "message:\($0.id)" })
}

@MainActor
@Test func conversationModelClearsGroupedActivityOnSessionChangeAndSearch() {
    let model = ConversationModel()
    model.apply(page([tool("a"), tool("b"), message("reply")]), older: false)
    #expect(model.rows.count == 2)
    model.begin(query: "body")
    #expect(model.rows.isEmpty)
    model.apply(page([tool("a"), tool("b")]), older: false)
    #expect(model.rows.count == 2)
    model.apply(page([message("new")], session: "other"), older: false)
    #expect(model.rows.map(\.id) == ["message:new"])
}

@Test func conversationMarkdownUsesParagraphSpacingAndPreservesCodeBlankLines() {
    let blocks = ConversationMarkdown.blocks("\nFirst\n\n\nSecond\n\n```\na\n\n  b\n```\n")
    #expect(blocks.count == 3)
    #expect(blocks[0].text == "First")
    #expect(blocks[1].text == "Second")
    #expect(blocks[2].text == "a\n\n  b")
    #expect(blocks[2].code)
}

@Test func conversationSearchHighlightPreservesTextAndLinks() throws {
    let original = try AttributedString(markdown: "[Résumé](https://example.com) RÉSUMÉ")
    let highlighted = ConversationMarkdown.highlighted(original, query: "résumé")
    #expect(String(highlighted.characters) == String(original.characters))
    #expect(highlighted.runs.filter { $0.backgroundColor != nil }.count == 2)
    #expect(highlighted.runs.first?.link == URL(string: "https://example.com"))
    #expect(ConversationMarkdown.highlighted(original, query: "") == original)
}

@MainActor
@Test func conversationSearchKeepsNewMatchesVisibleAfterScanningOlderPages() async {
    let model = ConversationModel()
    model.begin(query: "body")
    model.apply(page([tool("new", call: "two")]), older: false)
    await model.fetch(older: true) { before, after, query in
        #expect(before == "older" && after == nil && query == "body")
        return page([tool("old", call: "one")], before: nil, after: "oldEnd")
    }
    await model.fetch { before, after, query in
        #expect(before == nil && after == "latest" && query == "body")
        return page([tool("result", call: "two", result: true)], before: "unused", after: "newEnd")
    }
    #expect(model.before == nil)
    #expect(model.rows.map(\.id) == ["message:old", "message:new", "message:result"])
    #expect(model.rows.allSatisfy { $0.message != nil })
}

@MainActor
@Test func conversationOlderLoadingFeedbackLastsUntilTheRequestFinishes() async {
    let model = ConversationModel()
    model.apply(page([message("current")]), older: false)
    var pending: CheckedContinuation<HistoryPage, Never>?
    let task = Task {
        await model.fetch(older: true) { _, _, _ in
            await withCheckedContinuation { pending = $0 }
        }
    }
    while pending == nil { await Task.yield() }
    #expect(model.loadingOlder)
    pending?.resume(returning: page([message("older")], before: nil))
    await task.value
    #expect(!model.loadingOlder)
    #expect(model.messages.map(\.id) == ["older", "current"])
    await model.fetch { _, _, _ in
        #expect(model.loading)
        #expect(!model.loadingOlder)
        return page([], before: nil)
    }
}
