import CollieCore
import Foundation
import Observation

/// Screens for the Agents grid. Only while `run` runs (grid on screen, app active), each visible
/// card of a connected machine gets one `agent.read` when it first shows and at once when its
/// status changes; only a working or unknown agent, or one whose last read failed, is read again
/// every `interval`. collied audits every read, so a screen that cannot change is not re-read.
/// A watched agent's output already in the core is used instead of a read.
@MainActor
@Observable
final class PreviewModel {
    static let interval = Duration.seconds(5)
    static let tick = Duration.seconds(1)
    /// A 120 pt card shows about 20 rows at font size 5. herdr counts rows at the Mac pane
    /// width, so a card wider than a narrow pane rejoins them into fewer. The input box and
    /// status lines `card` crops off take about 10 more.
    static let lines: UInt16 = 60

    private(set) var screens: [AgentRoute: String] = [:]

    @ObservationIgnored private var core: (any AgentCore)?
    @ObservationIgnored private let now: () -> ContinuousClock.Instant
    @ObservationIgnored private var visible: Set<AgentRoute> = []
    @ObservationIgnored private var connected: Set<String> = []
    @ObservationIgnored private var statuses: [AgentRoute: (status: AgentState, since: UInt64)] = [:]
    @ObservationIgnored private var claude: Set<AgentRoute> = []
    @ObservationIgnored private var readAt: [AgentRoute: ContinuousClock.Instant] = [:]
    @ObservationIgnored private var fresh: Set<AgentRoute> = []
    @ObservationIgnored private var reading: Set<AgentRoute> = []
    @ObservationIgnored private var runs = 0

    init(now: @escaping () -> ContinuousClock.Instant = { .now }) {
        self.now = now
    }

    func run(core: any AgentCore) async {
        runs += 1
        let run = runs
        self.core = core
        // A restarted run can begin before the cancelled one has ended.
        defer { if runs == run { self.core = nil } }
        while !Task.isCancelled {
            tick()
            try? await Task.sleep(for: Self.tick)
        }
    }

    func appeared(_ route: AgentRoute) {
        visible.insert(route)
    }

    func disappeared(_ route: AgentRoute) {
        visible.remove(route)
    }

    @discardableResult
    func update(_ entries: [MachineFlockEntry]) -> Task<Void, Never>? {
        connected = Set(entries.filter { $0.flock?.link == .connected }.map(\.id))
        var next: [AgentRoute: (status: AgentState, since: UInt64)] = [:]
        claude = []
        for entry in entries {
            for agent in entry.flock?.agents ?? [] {
                let route = AgentRoute(machineId: entry.id, terminalId: agent.terminalId)
                next[route] = (agent.status, agent.statusSinceMs)
                if agent.kind == "claude" { claude.insert(route) }
            }
        }
        // `since` also moves on a change that came and went between two flock polls.
        for (route, status) in next where statuses[route].map({ $0.status != status.status || $0.since != status.since }) == true {
            readAt[route] = nil
            fresh.remove(route)
        }
        statuses = next
        readAt = readAt.filter { next[$0.key] != nil }
        fresh = fresh.filter { next[$0] != nil }
        if screens.keys.contains(where: { next[$0] == nil }) {
            screens = screens.filter { next[$0.key] != nil }
        }
        return tick()
    }

    /// The returned task ends when the reads it started have answered.
    @discardableResult
    func tick() -> Task<Void, Never>? {
        guard let core else { return nil }
        let now = now()
        let due = visible.filter { route in
            guard connected.contains(route.machineId), let status = statuses[route]?.status, !reading.contains(route) else { return false }
            guard let at = readAt[route] else { return true }
            return now - at >= Self.interval && ([.working, .unknown].contains(status) || !fresh.contains(route))
        }
        var reads: [AgentRoute] = []
        for route in due {
            readAt[route] = now
            if let output = core.agentView(machineId: route.machineId, terminalId: route.terminalId, afterRevision: 0)?.output {
                show(output.ansi, for: route)
                fresh.insert(route)
            } else {
                reads.append(route)
            }
        }
        guard !reads.isEmpty else { return nil }
        reading.formUnion(reads)
        return Task {
            await withTaskGroup(of: (AgentRoute, String?).self) { group in
                for route in reads {
                    group.addTask {
                        let read = try? await core.agentRead(machineId: route.machineId, terminalId: route.terminalId, source: .recent, lines: Self.lines)
                        return (route, read?.ansi)
                    }
                }
                for await (route, ansi) in group {
                    reading.remove(route)
                    if let ansi, statuses[route] != nil {
                        show(ansi, for: route)
                        fresh.insert(route)
                    }
                }
            }
        }
    }

    private func show(_ ansi: String, for route: AgentRoute) {
        let ansi = claude.contains(route) ? Self.card(ansi) : ansi
        if screens[route] != ansi { screens[route] = ansi }
    }

    /// Drops Claude Code's input box, the notification row above it and the status lines under it, and
    /// keeps the task and workflow progress list below those. The box is found as collied's
    /// `draft::parse` finds it: the last block between two rules drawn from column 0 whose first row
    /// starts at column 0 and whose other rows are indented by two spaces. Only indented lines may
    /// follow it, so a box an earlier session left above a newer screen does not count. A screen
    /// without one, such as a dialog in its place, is kept whole.
    nonisolated static func card(_ ansi: String) -> String {
        let lines = ansi.split(omittingEmptySubsequences: false, whereSeparator: \.isNewline)
        let plain = lines.map(plainText)
        let rules = plain.indices.filter { plain[$0].hasPrefix("─") && isRule(plain[$0]) }
        let blank = { (line: String) in line.allSatisfy(\.isWhitespace) }
        let indented = { (line: String) in line.hasPrefix("  ") || blank(line) }
        let box = Array(zip(rules, rules.dropFirst())).last { top, bottom in
            bottom > top + 1 && plain[top + 1].first.map { !$0.isWhitespace } == true
                && plain[top + 2..<bottom].allSatisfy(indented) && plain[(bottom + 1)...].allSatisfy(indented)
        }
        guard case let (top, bottom)? = box else { return ansi }
        // Claude Code draws its notifications right-aligned, two columns short of the rule, over the
        // blank row above the box.
        let notification = top > 0 && plain[top - 1].hasPrefix("  ") && !blank(plain[top - 1])
            && plain[top - 1].count == plain[top].count - 2
        guard let last = plain[..<(notification ? top - 1 : top)].lastIndex(where: { !blank($0) }) else { return ansi }
        let card = String(ansi[..<lines[last].endIndex])
        // The progress list sits a blank row below the status lines, its rows like "  ◯ name  ▰▱  1/2 · 3m".
        let tail = plain.indices[(bottom + 1)...]
        guard let first = tail.first(where: { (blank(plain[$0 - 1]) && !blank(plain[$0])) || plain[$0].contains(#/^  \S \S.* \d+/\d+ · /#) }),
            let final = tail.last(where: { !blank(plain[$0]) })
        else { return card }
        return card + ansi[lines[last].endIndex..<lines[last + 1].startIndex] + ansi[lines[first].startIndex..<lines[final].endIndex]
    }

    /// collied sends only plain SGR escapes.
    private nonisolated static func plainText(_ line: Substring) -> String {
        var text = String.UnicodeScalarView()
        var escape = false
        for c in line.unicodeScalars {
            if escape {
                escape = c != "m"
            } else if c == "\u{1b}" {
                escape = true
            } else {
                text.append(c)
            }
        }
        return String(text)
    }

    private nonisolated static func isRule(_ line: String) -> Bool {
        let dashes = line.prefix { $0 == "─" }.count
        return dashes >= 3 || line.dropFirst(dashes).allSatisfy(\.isWhitespace)
    }
}
