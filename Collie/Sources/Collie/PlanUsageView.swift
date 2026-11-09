import CollieCore
import SwiftUI

/// The Agents tab's Usage view: under each machine's name, a card with the plan usage collied last recorded,
/// then one with its tokens and estimated cost.
struct UsageSections: View {
    let entries: [MachineFlockEntry]
    let core: (any FlockCore)?
    var refresh = 0
    @Environment(\.dynamicTypeSize) private var dynamicTypeSize

    var body: some View {
        if entries.isEmpty {
            NoMachines()
        }
        ForEach(entries) { entry in
            let planUsage = entry.flock?.planUsage
            Group {
                Section {
                    if let usage = planUsage {
                        TimelineView(.everyMinute) { context in
                            let codex = usage.codex.flatMap { $0.resetsAtMs > context.date.unixMs ? $0 : nil }
                            let copilot = usage.copilot.flatMap { $0.resetsAtMs > context.date.unixMs ? $0 : nil }
                            VStack(alignment: .leading, spacing: 20) {
                                if usage.fiveHour != nil || usage.sevenDay != nil {
                                    SubscriptionUsage(usage: usage, waiting: Self.waiting(entry.flock?.agents ?? []), now: context.date)
                                    StopAtLimitToggle(machineId: entry.machine.id, on: entry.flock?.stopAtLimit ?? false, core: core)
                                }
                                if let codex {
                                    CodexSubscriptionUsage(usage: codex, now: context.date)
                                }
                                if let copilot {
                                    CopilotSubscriptionUsage(usage: copilot, now: context.date)
                                }
                            }
                            .padding(.vertical, 4)
                        }
                    }
                } header: {
                    // With one machine paired its name says nothing, as on Agents, and each stale reading says its own age.
                    // A TimelineView showing nothing still made a header, which set the first card off Agents' first card.
                    if entries.count > 1 {
                        TimelineView(.everyMinute) { context in
                            let layout = dynamicTypeSize.isAccessibilitySize ? AnyLayout(VStackLayout(alignment: .leading, spacing: 2)) : AnyLayout(HStackLayout(alignment: .firstTextBaseline))
                            // The age shows only once no reading is current.
                            let fresh = planUsage.map { !$0.readingsStale(now: context.date) } ?? false
                            layout {
                                MachineName(machine: entry.machine).font(.footnote.weight(.semibold))
                                if !dynamicTypeSize.isAccessibilitySize { Spacer() }
                                if !fresh {
                                    Text(planUsage?.age(now: context.date) ?? "No plan usage recorded").font(.footnote)
                                }
                            }
                            .textCase(nil)
                            .accessibilityElement(children: .combine)
                            .accessibilityLabel(fresh ? entry.machine.label : "\(entry.machine.label), \(planUsage?.spokenAge(now: context.date) ?? "no plan usage recorded")")
                        }
                    } else if planUsage == nil {
                        Text("No plan usage recorded").font(.footnote).textCase(nil)
                    }
                }
                Section {
                    UsageCostsCard(core: core, machineId: entry.machine.id, link: entry.flock?.link, refresh: refresh)
                }
            }
            .listSectionSpacing(.compact)
            .opacity(entry.linkDown ? 0.6 : 1)
        }
    }
}

extension UsageSections {
    /// Agents the machine holds stopped at each limit, by `UsageLimit.label`.
    static func waiting(_ agents: [AgentSummary]) -> [String: Int] {
        agents.reduce(into: [:]) { counts, agent in
            switch agent.planLimit?.window {
            case .fiveHour: counts["5h", default: 0] += 1
            case .sevenDay: counts["7d", default: 0] += 1
            case .unknown, nil: break
            }
        }
    }
}

/// Stored on the machine, so every phone shows the same; the next snapshot confirms a change.
private struct StopAtLimitToggle: View {
    let machineId: String
    let on: Bool
    let core: (any FlockCore)?
    @State private var asked: Bool?
    @State private var failure: String?

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            Toggle("Stop at the plan limit", isOn: Binding(get: { asked ?? on }, set: set))
                .font(.subheadline)
                .disabled(core == nil)
            Text(failure ?? "Agents wait for the reset instead of using extra usage.")
                .font(.caption)
                .foregroundStyle(.metadata)
        }
        .onChange(of: on) { asked = nil }
    }

    private func set(_ enabled: Bool) {
        guard let core else { return }
        asked = enabled
        Task {
            do {
                try await core.setStopAtLimit(machineId: machineId, enabled: enabled)
                failure = nil
            } catch {
                asked = nil
                failure = AgentModel.message(for: error)
            }
        }
    }
}

private struct CodexSubscriptionUsage: View {
    let usage: CodexUsage
    let now: Date

    var body: some View {
        let percent = usage.percent
        VStack(alignment: .leading, spacing: 10) {
            UsageHeader(image: "Codex", title: "Codex", recordedMs: usage.isStale(now: now) ? usage.recordedMs : nil, now: now)
            UsageLimitRow(
                title: "Monthly limit", figure: "\(usage.used.formatted()) / \(usage.limit.formatted())",
                used: percent, elapsed: usage.elapsed(now: now), resets: "credits · \(usage.resetsLine)"
            )
            .accessibilityElement(children: .ignore)
            .accessibilityLabel("Codex monthly usage limit, \(usage.used) of \(usage.limit) credits used, \(percent) percent, \(Int((usage.elapsed(now: now) * 100).rounded())) percent of the month elapsed")
        }
    }
}

private struct CopilotSubscriptionUsage: View {
    let usage: CopilotUsage
    let now: Date

    var body: some View {
        let percent = usage.percent
        VStack(alignment: .leading, spacing: 10) {
            UsageHeader(image: "Copilot", title: "Copilot", recordedMs: usage.isStale(now: now) ? usage.recordedMs : nil, now: now)
            UsageLimitRow(
                title: "Monthly premium usage",
                figure: usage.unlimited ? "Unlimited" : "\(usage.used.formatted()) / \(usage.limit.formatted())",
                used: usage.unlimited ? nil : UInt8(min(100, percent)), elapsed: usage.elapsed(now: now),
                resets: usage.unlimited ? usage.resetsLine : "\(usage.unit) · \(usage.resetsLine)"
            )
            .accessibilityElement(children: .ignore)
            .accessibilityLabel(
                usage.unlimited
                    ? "Copilot monthly premium usage, unlimited, \(usage.resetsLine)"
                    : "Copilot monthly premium usage, \(usage.used) of \(usage.limit) \(usage.unit) used, \(percent) percent, \(Int((usage.elapsed(now: now) * 100).rounded())) percent of the month elapsed"
            )
        }
    }
}

private struct SubscriptionUsage: View {
    let usage: PlanUsage
    let waiting: [String: Int]
    let now: Date

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            UsageHeader(image: "Claude", title: "Claude Code", recordedMs: usage.isStale(now: now) ? usage.recordedMs : nil, now: now)
            VStack(alignment: .leading, spacing: 16) {
                ForEach(usage.limits(now: now), id: \.label) { limit in
                    let held = waiting[limit.label].map { $0 == 1 ? "1 agent waits" : "\($0) agents wait" }
                    UsageLimitRow(
                        title: limit.title, figure: "\(limit.used)%", used: limit.used, elapsed: limit.elapsed,
                        resets: [limit.resetsLine(now: now), held].compactMap { $0 }.joined(separator: " · ")
                    )
                    .accessibilityElement(children: .ignore)
                    .accessibilityLabel([limit.spokenLine(now: now), held].compactMap { $0 }.joined(separator: ", "))
                }
            }
        }
    }
}

/// The figure leads, the bar follows, the reset time sits under it.
private struct UsageLimitRow: View {
    let title: String
    let figure: String
    let used: UInt8?
    let elapsed: Double
    let resets: String
    @Environment(\.dynamicTypeSize) private var dynamicTypeSize

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            let layout = dynamicTypeSize.isAccessibilitySize ? AnyLayout(VStackLayout(alignment: .leading, spacing: 2)) : AnyLayout(HStackLayout(alignment: .firstTextBaseline))
            layout {
                Text(title)
                if !dynamicTypeSize.isAccessibilitySize { Spacer(minLength: 8) }
                Text(figure).fontWeight(.semibold).monospacedDigit().layoutPriority(1)
                    .foregroundStyle(used.map { $0 >= 80 ? ContextRing.tone(100 - $0) : .primary } ?? .primary)
            }
            .font(.subheadline)
            if let used {
                UsageBar(used: used, elapsed: elapsed)
            }
            Text(resets).font(.caption.monospacedDigit()).foregroundStyle(.metadata)
        }
    }
}

private struct UsageHeader: View {
    let image: String
    let title: String
    /// Set when the reading is stale, to say how old it is.
    let recordedMs: UInt64?
    let now: Date
    @ScaledMetric(relativeTo: .headline) private var logo: CGFloat = 20

    var body: some View {
        HStack(spacing: 8) {
            Image(image)
                .resizable()
                .scaledToFit()
                .frame(width: logo, height: logo)
                .accessibilityHidden(true)
            Text(title).font(.headline)
            Spacer(minLength: 8)
            if let recordedMs {
                Text("Updated \(PlanUsage.age(sinceMs: recordedMs, now: now))").font(.caption).foregroundStyle(.metadata)
            }
        }
    }
}

/// Percent used, in the accent until the context ring's warning tiers, with a tick where the window's elapsed share is.
private struct UsageBar: View {
    let used: UInt8
    let elapsed: Double

    var body: some View {
        GeometryReader { geo in
            let width = geo.size.width
            ZStack(alignment: .leading) {
                Capsule().fill(.quaternary).frame(height: 4)
                Capsule().fill(used < 80 ? AnyShapeStyle(.tint) : AnyShapeStyle(ContextRing.tone(100 - used))).frame(width: width * Double(used) / 100, height: 4)
                Capsule()
                    .fill(.secondary)
                    .frame(width: 2, height: 12)
                    .offset(x: min(max(width * elapsed - 1, 0), width - 2))
            }
            .frame(maxHeight: .infinity)
        }
        .frame(minWidth: 60, maxWidth: .infinity)
        .frame(height: 12)
    }
}
