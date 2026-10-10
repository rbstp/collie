import CollieCore
import Foundation
import Testing

@testable import Collie

private func message(_ id: String, _ text: String = "hello") -> HistoryMessage {
    HistoryMessage(id: id, role: "assistant", text: text, tool: nil, callId: nil, truncated: false)
}

private func page(_ messages: [HistoryMessage], session: String = "s1", reset: Bool = false, before: String? = "older", after: String = "latest") -> HistoryPage {
    HistoryPage(session: session, available: true, reset: reset, messages: messages, before: before, after: after, more: false, omitted: false)
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
    model.apply(HistoryPage(session: nil, available: false, reset: false, messages: [], before: nil, after: nil, more: false, omitted: false), older: false)
    #expect(model.available == false)
    #expect(model.messages.isEmpty)
    #expect(model.after == nil)
    model.apply(page([message("recovered")]), older: false)
    #expect(model.available == true)
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
