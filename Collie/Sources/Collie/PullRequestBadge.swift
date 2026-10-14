import CollieCore
import SwiftUI

struct PullRequestBadge: View {
    let pullRequest: PullRequestSummary
    let watch: PrWatchSummary?
    var compact = false
    @Environment(\.openURL) private var openURL

    var body: some View {
        let url = Self.url(pullRequest)
        Button {
            if let url { openURL(url) }
        } label: {
            HStack(spacing: 3) {
                Image(systemName: Self.symbol(pullRequest.state))
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
        .buttonStyle(.borderless)
        .fixedSize()
        .disabled(url == nil)
        .accessibilityLabel(Self.spoken(pullRequest, watch: watch))
        .accessibilityHint("Opens the pull request")
    }

    static func url(_ pullRequest: PullRequestSummary) -> URL? {
        guard let url = URL(string: pullRequest.url), url.scheme?.lowercased() == "https",
              url.host?.isEmpty == false else { return nil }
        return url
    }

    static func symbol(_ state: PullState) -> String {
        switch state {
        case .open, .unknown: "arrow.triangle.pull"
        case .draft: "circle.dashed"
        case .merged: "arrow.triangle.merge"
        case .closed: "xmark.circle"
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
