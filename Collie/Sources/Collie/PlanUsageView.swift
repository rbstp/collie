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
                                    SubscriptionUsage(usage: usage, now: context.date)
                                }
                                if let codex {
                                    CodexSubscriptionUsage(usage: codex, now: context.date)
                                }
                                if let copilot {
                                    CopilotSubscriptionUsage(usage: copilot, now: context.date)
                                }
                                if !usage.limits(now: context.date).isEmpty || codex != nil || copilot.map({ !$0.unlimited }) == true {
                                    Text("Ticks mark how much of each window has passed.")
                                        .font(.caption)
                                        .foregroundStyle(.metadata)
                                }
                            }
                            .padding(.vertical, 4)
                        }
                    }
                } header: {
                    TimelineView(.everyMinute) { context in
                        let layout = dynamicTypeSize.isAccessibilitySize ? AnyLayout(VStackLayout(alignment: .leading, spacing: 2)) : AnyLayout(HStackLayout(alignment: .firstTextBaseline))
                        layout {
                            MachineName(machine: entry.machine).font(.footnote.weight(.semibold))
                            if !dynamicTypeSize.isAccessibilitySize { Spacer() }
                            Text(entry.flock?.planUsage?.age(now: context.date) ?? "No plan usage recorded").font(.footnote)
                        }
                        .textCase(nil)
                        .accessibilityElement(children: .combine)
                        .accessibilityLabel("\(entry.machine.label), \(entry.flock?.planUsage?.spokenAge(now: context.date) ?? "no plan usage recorded")")
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
    let now: Date

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            UsageHeader(image: "Claude", title: "Claude Code", recordedMs: usage.isStale(now: now) ? usage.recordedMs : nil, now: now)
            VStack(alignment: .leading, spacing: 16) {
                ForEach(usage.limits(now: now), id: \.label) { limit in
                    UsageLimitRow(title: limit.title, figure: "\(limit.used)%", used: limit.used, elapsed: limit.elapsed, resets: limit.resetsLine(now: now))
                        .accessibilityElement(children: .ignore)
                        .accessibilityLabel(limit.spokenLine(now: now))
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
