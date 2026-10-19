import Charts
import CollieCore
import SwiftUI

@MainActor
@Observable
final class UsageCostsModel {
    enum Phase: Equatable {
        case loading
        case loaded(UsageCosts)
        case outdated
        case failed(String)
    }

    private(set) var phase: Phase = .loading
    private var range: CostRange?

    /// Reads again while collied is still scanning, so the totals grow as it goes. A failed read
    /// keeps the figures of the same range, and retries while they are still partial.
    func load(
        range: CostRange,
        offsetMinutes: Int16 = Int16(TimeZone.current.secondsFromGMT() / 60),
        every interval: Duration = .seconds(2),
        fetch: (CostRange, Int16) async throws -> UsageCosts
    ) async {
        if self.range != range {
            self.range = range
            phase = .loading
        }
        while !Task.isCancelled {
            do {
                let costs = try await fetch(range, offsetMinutes)
                // A read cannot be cancelled once sent, so a late reply for a replaced load is dropped.
                guard !Task.isCancelled, self.range == range else { return }
                phase = .loaded(costs)
                guard costs.scanning else { return }
            } catch CoreError.NotImplemented {
                guard !Task.isCancelled, self.range == range else { return }
                phase = .outdated
                return
            } catch {
                guard !Task.isCancelled, self.range == range else { return }
                guard case .loaded(let shown) = phase else {
                    phase = .failed(AgentModel.message(for: error))
                    return
                }
                guard shown.scanning else { return }
            }
            try? await Task.sleep(for: interval)
        }
    }
}

extension CostRange {
    var title: String {
        switch self {
        case .today: "Today"
        case .week: "7 days"
        case .month: "30 days"
        }
    }
}

extension ModelCost {
    var agentName: String {
        switch agent {
        case "claude": "Claude Code"
        case "codex": "Codex"
        case "copilot": "Copilot CLI"
        default: agent
        }
    }

    var costText: String { costUsd.map(UsageCostsCard.dollars) ?? "No price" }
}

extension CostTokens {
    var total: UInt64 {
        [output, cacheRead, cacheWrite].reduce(input) { sum, n in
            let (added, overflow) = sum.addingReportingOverflow(n)
            return overflow ? .max : added
        }
    }
}

extension DayCost {
    /// Midnight of `date` in the phone's calendar, which collied used to cut the days.
    var day: Date? {
        let parts = date.split(separator: "-").compactMap { Int($0) }
        guard parts.count == 3 else { return nil }
        return Calendar.current.date(from: DateComponents(year: parts[0], month: parts[1], day: parts[2]))
    }
}

/// Estimated tokens and cost from the machine's transcripts, below its plan usage.
struct UsageCostsCard: View {
    let core: (any AgentCore)?
    let machineId: String
    var link: LinkPhase?
    var refresh = 0
    @State private var range: CostRange = .week
    @State private var model = UsageCostsModel()

    nonisolated static func dollars(_ value: Double) -> String {
        value.formatted(.currency(code: "USD").precision(.fractionLength(value > 0 && value < 0.01 ? 4 : 2)))
    }

    nonisolated static func tokens(_ value: UInt64) -> String {
        value.formatted(.number.notation(.compactName))
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack {
                Text("Tokens and cost").font(.headline)
                Spacer()
                if case .loaded(let costs) = model.phase, costs.scanning {
                    ProgressView().controlSize(.small)
                    Text("Scanning").font(.caption).foregroundStyle(.secondary)
                }
            }
            .accessibilityElement(children: .combine)
            GlassPicker(title: "Range", selection: $range, options: [CostRange.today, .week, .month].map { ($0.title, $0) })
            switch model.phase {
            case .loading:
                ProgressView().frame(maxWidth: .infinity)
            case .outdated:
                Text("Update collied on this machine to see tokens and cost.").font(.subheadline).foregroundStyle(.secondary)
            case .failed(let message):
                Text(message).font(.subheadline).foregroundStyle(.secondary)
            case .loaded(let costs):
                CostsDetail(costs: costs)
            }
        }
        .padding(.vertical, 4)
        .task(id: CostsRead(range: range, link: link, refresh: refresh)) {
            guard let core else { return }
            let machineId = machineId
            await model.load(range: range) { range, offset in
                try await core.usageCosts(machineId: machineId, range: range, utcOffsetMinutes: offset)
            }
        }
    }
}

private struct CostsRead: Equatable {
    let range: CostRange
    let link: LinkPhase?
    let refresh: Int
}

private struct CostsDetail: View {
    let costs: UsageCosts
    @Environment(\.dynamicTypeSize) private var dynamicTypeSize

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            VStack(alignment: .leading, spacing: 2) {
                Text(UsageCostsCard.dollars(costs.costUsd)).font(.title2.weight(.semibold).monospacedDigit())
                Text("Estimated at API prices · cache saved \(UsageCostsCard.dollars(costs.cacheSavingsUsd))")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            .accessibilityElement(children: .combine)
            Grid(alignment: .leading, horizontalSpacing: 16, verticalSpacing: 4) {
                GridRow {
                    TokenFigure(title: "Input", value: costs.tokens.input)
                    TokenFigure(title: "Output", value: costs.tokens.output)
                }
                GridRow {
                    TokenFigure(title: "Cache read", value: costs.tokens.cacheRead)
                    TokenFigure(title: "Cache write", value: costs.tokens.cacheWrite)
                }
            }
            if costs.days.count > 1 {
                Chart(costs.days.compactMap { day in day.day.map { (day: $0, cost: day.costUsd) } }, id: \.day) { bar in
                    BarMark(x: .value("Day", bar.day, unit: .day), y: .value("Cost", bar.cost))
                }
                .chartYAxis {
                    AxisMarks { value in
                        AxisGridLine()
                        AxisValueLabel { value.as(Double.self).map { Text(UsageCostsCard.dollars($0)) } }
                    }
                }
                .frame(height: 120)
                .accessibilityLabel("Daily cost")
            }
            if costs.models.isEmpty, !costs.scanning {
                Text("No tokens recorded in this range").font(.subheadline).foregroundStyle(.secondary)
            }
            ForEach(costs.models, id: \.self) { row in
                VStack(alignment: .leading, spacing: 2) {
                    let layout = dynamicTypeSize.isAccessibilitySize ? AnyLayout(VStackLayout(alignment: .leading, spacing: 2)) : AnyLayout(HStackLayout(alignment: .firstTextBaseline))
                    layout {
                        Text(row.model).font(.subheadline).lineLimit(dynamicTypeSize.isAccessibilitySize ? nil : 1).truncationMode(.middle)
                        if !dynamicTypeSize.isAccessibilitySize { Spacer() }
                        Text(row.costText).font(.subheadline.monospacedDigit()).foregroundStyle(row.costUsd == nil ? .secondary : .primary)
                    }
                    Text("\(row.agentName) · \(UsageCostsCard.tokens(row.tokens.total)) tokens")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
                .accessibilityElement(children: .combine)
            }
            if costs.models.contains(where: { $0.agent == "copilot" }) {
                Text("Copilot bills premium requests; its cost is an API-equivalent estimate.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
        }
    }
}

private struct TokenFigure: View {
    let title: String
    let value: UInt64

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            Text(UsageCostsCard.tokens(value)).font(.subheadline.monospacedDigit())
            Text(title).font(.caption).foregroundStyle(.secondary)
        }
        .accessibilityElement(children: .combine)
    }
}
