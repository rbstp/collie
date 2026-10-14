import CollieCore
import SwiftUI

struct PullRequestBadge: View {
    let pullRequest: PullRequestSummary
    let watch: PrWatchSummary?
    var compact = false
    var fill: ((String) -> Void)?
    @Environment(\.openURL) private var openURL
    @ScaledMetric(relativeTo: .caption2) private var iconSize = 11.0

    var body: some View {
        let url = Self.url(pullRequest)
        if let fill, let url {
            Menu {
                Button("Open PR", systemImage: "safari") { openURL(url) }
                ForEach(Self.prompts(pullRequest), id: \.title) { prompt in
                    Button(prompt.title) { fill(prompt.text) }
                }
            } label: {
                label
            }
            .buttonStyle(.borderless)
            .fixedSize()
            .accessibilityLabel(Self.spoken(pullRequest, watch: watch))
            .accessibilityHint("Shows pull request actions")
        } else {
            Button {
                if let url { openURL(url) }
            } label: {
                label
            }
            .buttonStyle(.borderless)
            .fixedSize()
            .disabled(url == nil)
            .accessibilityLabel(Self.spoken(pullRequest, watch: watch))
            .accessibilityHint("Opens the pull request")
        }
    }

    private var label: some View {
        HStack(spacing: 3) {
            Self.icon(pullRequest.state).resizable().scaledToFit().frame(width: iconSize, height: iconSize)
            Text(verbatim: "#\(pullRequest.number)").monospacedDigit()
            if pullRequest.state == .merged && !compact { Text("Merged") }
            if let check = Self.check(pullRequest) {
                Image(systemName: check.symbol).foregroundStyle(check.color)
            }
            if Self.watching(pullRequest, watch) {
                Image(systemName: "eye")
                if !compact { Text("Watching") }
            }
        }
        .font(.caption2.weight(.medium))
        .foregroundStyle(Self.color(pullRequest.state))
        .padding(.horizontal, 6)
        .padding(.vertical, 1)
        .background(Self.color(pullRequest.state).opacity(0.15), in: Capsule())
    }

    struct Prompt: Equatable {
        let title: String
        let text: String
    }

    /// Only the URL and the failing check names, never review or comment text.
    static func prompts(_ pullRequest: PullRequestSummary) -> [Prompt] {
        let url = pullRequest.url
        var prompts: [Prompt] = []
        if pullRequest.state == .open || pullRequest.state == .draft {
            if pullRequest.checks == .failing {
                let names = pullRequest.failingChecks.map { "\"\($0)\"" }.joined(separator: ", ")
                prompts.append(Prompt(
                    title: "Fix failing checks",
                    text: "Fix the failing checks of pull request \(url)" + (names.isEmpty ? "." : ": \(names).")
                ))
            }
            if pullRequest.remarks > 0 {
                prompts.append(Prompt(title: "Address review comments", text: "Address the review comments on pull request \(url)."))
            }
            if pullRequest.conflicting {
                prompts.append(Prompt(title: "Resolve conflicts", text: "Resolve the merge conflicts of pull request \(url) with its base branch."))
            }
        }
        prompts.append(Prompt(title: "Explain this PR", text: "Explain pull request \(url)."))
        return prompts
    }

    static func url(_ pullRequest: PullRequestSummary) -> URL? {
        guard let url = URL(string: pullRequest.url), url.scheme?.lowercased() == "https",
              url.host?.isEmpty == false else { return nil }
        return url
    }

    /// Octicons from GitHub (MIT, Resources/Octicons-LICENSE.txt) for the states GitHub draws with them.
    static func icon(_ state: PullState) -> Image {
        switch state {
        case .open, .unknown: Image("GitPullRequest")
        case .draft: Image(systemName: "circle.dashed")
        case .merged: Image("GitMerge")
        case .closed: Image(systemName: "xmark.circle")
        }
    }

    static func color(_ state: PullState) -> Color {
        switch state {
        case .open: .green
        case .draft, .unknown: .secondary
        case .merged: .purple
        case .closed: .red
        }
    }

    /// Checks only matter while the pull request can still change.
    static func check(_ pullRequest: PullRequestSummary) -> (symbol: String, color: Color, spoken: String)? {
        guard pullRequest.state == .open || pullRequest.state == .draft else { return nil }
        switch pullRequest.checks {
        case .passing: return ("checkmark", .green, "checks passing")
        case .failing: return ("xmark", .red, "checks failing")
        case .pending: return ("clock", .orange, "checks pending")
        case .noChecks, .unknown: return nil
        }
    }

    static func watching(_ pullRequest: PullRequestSummary, _ watch: PrWatchSummary?) -> Bool {
        watch?.url == pullRequest.url
    }

    static func spoken(_ pullRequest: PullRequestSummary, watch: PrWatchSummary? = nil) -> String {
        let state = switch pullRequest.state {
        case .open: "open"
        case .draft: "draft"
        case .merged: "merged"
        case .closed: "closed"
        case .unknown: "state unknown"
        }
        let watched = watching(pullRequest, watch) ? watch.map { "watching every \($0.everyMinutes) minutes" } : nil
        return (["Pull request \(pullRequest.number)", state] + [check(pullRequest)?.spoken, watched].compactMap { $0 })
            .joined(separator: ", ")
    }

    static let intervals: [UInt32] = [2, 5, 15, 30]

    static func archiveFirst(_ agent: AgentSummary?) -> Bool {
        agent?.pullRequest?.state == .merged
    }

    /// The session menu's item: watch a Claude Code agent's open or draft pull request, or
    /// stop any watch on the terminal, linked or not.
    static func watchMenu(_ agent: AgentSummary?) -> WatchMenu? {
        guard let agent else { return nil }
        if let watch = agent.prWatch { return .stop("Stop watching #\(watch.number), every \(watch.everyMinutes) min") }
        guard agent.kind == "claude", let pullRequest = agent.pullRequest,
              pullRequest.state == .open || pullRequest.state == .draft else { return nil }
        return .watch
    }

    enum WatchMenu: Equatable {
        case watch
        case stop(String)
    }
}
