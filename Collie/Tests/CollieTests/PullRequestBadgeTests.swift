import CollieCore
import Foundation
import SwiftUI
import Testing

@testable import Collie

private func pullRequest(
    _ state: PullState, _ checks: PullChecks, url: String = "https://github.com/rbstp/collie/pull/264",
    failing: [String] = [], remarks: UInt32 = 0, conflicting: Bool = false
) -> PullRequestSummary {
    PullRequestSummary(
        number: 264, state: state, checks: checks, url: url, failingChecks: failing, remarks: remarks, conflicting: conflicting
    )
}

@MainActor
@Test func badgeMenuOffersOnlyThePromptsThatMatchThePullRequest() {
    let url = "https://github.com/rbstp/collie/pull/264"
    let titles = { (pr: PullRequestSummary) in PullRequestBadge.prompts(pr).map(\.title) }
    #expect(titles(pullRequest(.open, .passing)) == ["Explain this PR"])
    #expect(titles(pullRequest(.open, .failing, failing: ["lint"], remarks: 2, conflicting: true))
            == ["Fix failing checks", "Address review comments", "Resolve conflicts", "Explain this PR"])
    #expect(titles(pullRequest(.draft, .pending, remarks: 1)) == ["Address review comments", "Explain this PR"])
    #expect(titles(pullRequest(.merged, .failing, failing: ["lint"], remarks: 2, conflicting: true)) == ["Explain this PR"])
    #expect(titles(pullRequest(.closed, .passing, conflicting: true)) == ["Explain this PR"])

    let prompts = PullRequestBadge.prompts(pullRequest(.open, .failing, failing: ["lint", "ios test"], remarks: 2, conflicting: true))
    #expect(prompts.map(\.text) == [
        "Fix the failing checks of pull request \(url): \"lint\", \"ios test\".",
        "Address the review comments on pull request \(url).",
        "Resolve the merge conflicts of pull request \(url) with its base branch.",
        "Explain pull request \(url).",
    ])
    #expect(PullRequestBadge.prompts(pullRequest(.open, .failing)).first?.text == "Fix the failing checks of pull request \(url).")
}

@MainActor
@Test func aBadgePromptFillsTheComposerWithoutSending() {
    let core = FakeCore()
    let model = agentModel(core)
    model.draft = "first"
    let text = PullRequestBadge.prompts(pullRequest(.open, .passing)).last!.text
    #expect(model.insert(PromptShortcut(text: text)))
    #expect(model.draft == "first\n\nExplain pull request https://github.com/rbstp/collie/pull/264.")
    #expect(core.snapshot.prompts.isEmpty)
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
    #expect(UIImage(named: "GitPullRequest") != nil)
    #expect(UIImage(named: "GitMerge") != nil)
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

@MainActor
@Test func sessionMenuPutsArchiveFirstOnceThePullRequestMerged() {
    #expect(PullRequestBadge.archiveFirst(agent(pullRequest: pullRequest(.merged, .passing))))
    #expect(PullRequestBadge.archiveFirst(agent(kind: "codex", pullRequest: pullRequest(.merged, .noChecks))))
    for state in [PullState.open, .draft, .closed, .unknown] {
        #expect(!PullRequestBadge.archiveFirst(agent(pullRequest: pullRequest(state, .passing))), "\(state)")
    }
    #expect(!PullRequestBadge.archiveFirst(agent(pullRequest: nil)))
    #expect(!PullRequestBadge.archiveFirst(nil))
}
