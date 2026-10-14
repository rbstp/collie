import CollieCore
import Foundation
import SwiftUI
import Testing

@testable import Collie

private func pullRequest(
    _ state: PullState, _ checks: PullChecks, url: String = "https://github.com/rbstp/collie/pull/264"
) -> PullRequestSummary {
    PullRequestSummary(number: 264, state: state, checks: checks, url: url)
}

@MainActor
@Test func badgeShowsStateAndChecksOfAnOpenPullRequest() {
    #expect(PullRequestBadge.spoken(pullRequest(.open, .failing)) == "Pull request 264, open, checks failing")
    #expect(PullRequestBadge.check(pullRequest(.open, .failing))?.symbol == "xmark")
    #expect(PullRequestBadge.check(pullRequest(.draft, .passing))?.symbol == "checkmark")
    #expect(PullRequestBadge.check(pullRequest(.open, .pending))?.symbol == "clock")
    #expect(PullRequestBadge.check(pullRequest(.open, .noChecks)) == nil)
    #expect(PullRequestBadge.spoken(pullRequest(.draft, .noChecks)) == "Pull request 264, draft")
}

@MainActor
@Test func badgeShowsMergedAndClosedWithoutChecks() {
    #expect(PullRequestBadge.spoken(pullRequest(.merged, .failing)) == "Pull request 264, merged")
    #expect(PullRequestBadge.spoken(pullRequest(.closed, .passing)) == "Pull request 264, closed")
    #expect(PullRequestBadge.check(pullRequest(.merged, .failing)) == nil)
    let symbols = [PullState.open, .draft, .merged, .closed].map(PullRequestBadge.symbol)
    #expect(Set(symbols).count == 4)
    for symbol in symbols {
        #expect(UIImage(systemName: symbol) != nil)
    }
    for check in [PullChecks.passing, .failing, .pending] {
        #expect(UIImage(systemName: PullRequestBadge.check(pullRequest(.open, check))!.symbol) != nil)
    }
}

@MainActor
@Test func badgeOpensOnlyHttpsLinks() {
    #expect(PullRequestBadge.url(pullRequest(.open, .passing))?.absoluteString == "https://github.com/rbstp/collie/pull/264")
    for url in ["http://github.com/o/r/pull/1", "javascript:alert(1)", "collie://agent", "https:///pull/1", ""] {
        #expect(PullRequestBadge.url(pullRequest(.open, .passing, url: url)) == nil, "\(url)")
    }
}

@MainActor
@Test func sessionMenuOffersWatchForAnOpenPullRequestAndStopWhileWatching() async {
    #expect(PullRequestBadge.watchItem(nil) == nil)
    #expect(PullRequestBadge.watchItem(pullRequest(.open, .pending))?.title == "Watch PR")
    #expect(PullRequestBadge.watchItem(pullRequest(.draft, .noChecks))?.watch == true)
    #expect(PullRequestBadge.watchItem(pullRequest(.merged, .passing)) == nil)
    #expect(PullRequestBadge.watchItem(pullRequest(.closed, .failing)) == nil)
    var watched = pullRequest(.open, .failing)
    watched.watching = true
    #expect(PullRequestBadge.watchItem(watched)?.title == "Stop watching")
    #expect(PullRequestBadge.watchItem(watched)?.watch == false)
    #expect(PullRequestBadge.spoken(watched) == "Pull request 264, open, checks failing, watching")

    let core = FakeCore()
    core.state.withLock { $0.kind = "claude" }
    let model = agentModel(core)
    await model.watchPullRequest(true)
    await model.watchPullRequest(false)
    #expect(core.snapshot.prWatches == ["m1 term_1 true", "m1 term_1 false"])
    core.set(error: .Rejected(message: "the agent has no linked pull request"))
    await model.watchPullRequest(true)
    #expect(model.notice != nil)
}
