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

private func agent(kind: String = "claude", pullRequest: PullRequestSummary?, watch: PrWatchSummary? = nil) -> AgentSummary {
    AgentSummary(
        terminalId: "term_1", workspaceId: "w1", kind: kind, name: nil, title: nil, status: .idle, statusSinceMs: 0,
        cwd: nil, lastLine: nil, pullRequest: pullRequest, prWatch: watch
    )
}

@MainActor
@Test func sessionMenuOffersWatchForAClaudePullRequestAndStopForAnyWatch() async {
    #expect(PullRequestBadge.watchMenu(nil) == nil)
    #expect(PullRequestBadge.watchMenu(agent(pullRequest: nil)) == nil)
    #expect(PullRequestBadge.watchMenu(agent(pullRequest: pullRequest(.open, .pending))) == .watch)
    #expect(PullRequestBadge.watchMenu(agent(pullRequest: pullRequest(.draft, .noChecks))) == .watch)
    #expect(PullRequestBadge.watchMenu(agent(kind: "codex", pullRequest: pullRequest(.open, .pending))) == nil)
    #expect(PullRequestBadge.watchMenu(agent(pullRequest: pullRequest(.merged, .passing))) == nil)
    #expect(PullRequestBadge.watchMenu(agent(pullRequest: pullRequest(.closed, .failing))) == nil)
    #expect(PullRequestBadge.intervals == [2, 5, 15, 30])

    let watch = PrWatchSummary(number: 264, url: "https://github.com/rbstp/collie/pull/264", everyMinutes: 15)
    #expect(PullRequestBadge.watchMenu(agent(pullRequest: pullRequest(.open, .failing), watch: watch))
            == .stop("Stop watching #264, every 15 min"))
    let older = PrWatchSummary(number: 200, url: "https://github.com/rbstp/collie/pull/200", everyMinutes: 2)
    #expect(PullRequestBadge.watchMenu(agent(pullRequest: pullRequest(.open, .failing), watch: older))
            == .stop("Stop watching #200, every 2 min"), "a watch on another pull request can still be stopped")
    #expect(PullRequestBadge.watchMenu(agent(pullRequest: nil, watch: older)) == .stop("Stop watching #200, every 2 min"))
    #expect(PullRequestBadge.spoken(pullRequest(.open, .failing), watch: watch)
            == "Pull request 264, open, checks failing, watching every 15 minutes")
    #expect(!PullRequestBadge.watching(pullRequest(.open, .failing), older))

    let core = FakeCore()
    core.state.withLock { $0.kind = "claude" }
    let model = agentModel(core)
    await model.watchPullRequest(everyMinutes: 15)
    await model.watchPullRequest(everyMinutes: nil)
    #expect(core.snapshot.prWatches == ["m1 term_1 15", "m1 term_1 stop"])
    core.set(error: .Rejected(message: "gh is not signed in; run `gh auth login` on the machine"))
    await model.watchPullRequest(everyMinutes: 2)
    #expect(model.notice != nil)
}
