import CollieCore
import SwiftUI

/// The Agents tab's Usage view: one card per machine with the plan usage collied last recorded,
/// then its tokens and estimated cost.
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
            Section {
                TimelineView(.everyMinute) { context in
                    let layout = dynamicTypeSize.isAccessibilitySize ? AnyLayout(VStackLayout(alignment: .leading, spacing: 2)) : AnyLayout(HStackLayout())
                    layout {
                        MachineName(machine: entry.machine).font(.subheadline.weight(.semibold)).foregroundStyle(.primary)
                        if !dynamicTypeSize.isAccessibilitySize { Spacer() }
                        Text(entry.flock?.planUsage?.age(now: context.date) ?? "No plan usage recorded").font(.caption).foregroundStyle(.secondary)
                    }
                    .accessibilityElement(children: .combine)
                    .accessibilityLabel("\(entry.machine.label), \(entry.flock?.planUsage?.spokenAge(now: context.date) ?? "no plan usage recorded")")
                }
                if let usage = entry.flock?.planUsage {
                    TimelineView(.everyMinute) { context in
                        VStack(alignment: .leading, spacing: 12) {
                            if usage.fiveHour != nil || usage.sevenDay != nil {
                                SubscriptionUsage(usage: usage, now: context.date)
                            }
                            if let codex = usage.codex, codex.resetsAtMs > context.date.unixMs {
                                CodexSubscriptionUsage(usage: codex, now: context.date)
                            }
                            if let copilot = usage.copilot, copilot.resetsAtMs > context.date.unixMs {
                                CopilotSubscriptionUsage(usage: copilot, now: context.date)
                            }
                        }
                    }
                }
                UsageCostsCard(core: core, machineId: entry.machine.id, link: entry.flock?.link, refresh: refresh)
            }
            .opacity(entry.linkDown ? 0.5 : 1)
        }
    }
}

private struct CodexSubscriptionUsage: View {
    let usage: CodexUsage
    let now: Date
    @ScaledMetric(relativeTo: .headline) private var logo: CGFloat = 22

    var body: some View {
        let percent = usage.percent
        VStack(alignment: .leading, spacing: 8) {
            HStack(spacing: 12) {
                Image("Codex")
                    .resizable()
                    .scaledToFit()
                    .frame(width: logo, height: logo)
                    .padding(logo / 3)
                    .background(.quaternary.opacity(0.6), in: RoundedRectangle(cornerRadius: logo / 2, style: .continuous))
                    .accessibilityHidden(true)
                Text("Codex").font(.headline)
            }
            Text("Monthly usage limit").font(.subheadline)
            UsageBar(used: percent, elapsed: usage.elapsed(now: now))
            HStack {
                Text("\(usage.used.formatted()) / \(usage.limit.formatted()) credits")
                Spacer()
                Text(usage.resetsLine)
            }
            .font(.caption.monospacedDigit())
            .foregroundStyle(.secondary)
        }
        .padding(.vertical, 4)
        .opacity(usage.isStale(now: now) ? 0.6 : 1)
        .accessibilityElement(children: .combine)
        .accessibilityLabel("Codex monthly usage limit, \(usage.used) of \(usage.limit) credits used, \(percent) percent, \(Int((usage.elapsed(now: now) * 100).rounded())) percent of the month elapsed")
    }
}

private struct CopilotSubscriptionUsage: View {
    let usage: CopilotUsage
    let now: Date
    @Environment(\.dynamicTypeSize) private var dynamicTypeSize
    @ScaledMetric(relativeTo: .headline) private var logo: CGFloat = 22

    var body: some View {
        let percent = usage.percent
        VStack(alignment: .leading, spacing: 8) {
            HStack(spacing: 12) {
                Image("Copilot")
                    .resizable()
                    .scaledToFit()
                    .frame(width: logo, height: logo)
                    .padding(logo / 3)
                    .background(.quaternary.opacity(0.6), in: RoundedRectangle(cornerRadius: logo / 2, style: .continuous))
                    .accessibilityHidden(true)
                Text("Copilot").font(.headline)
            }
            Text("Monthly premium usage").font(.subheadline)
            if !usage.unlimited {
                UsageBar(used: UInt8(min(100, percent)), elapsed: usage.elapsed(now: now))
            }
            let layout = dynamicTypeSize.isAccessibilitySize ? AnyLayout(VStackLayout(alignment: .leading, spacing: 2)) : AnyLayout(HStackLayout())
            layout {
                Text(usage.unlimited ? "Unlimited" : "\(usage.used.formatted()) / \(usage.limit.formatted()) \(usage.unit)")
                if !dynamicTypeSize.isAccessibilitySize { Spacer() }
                Text(usage.resetsLine)
            }
            .font(.caption.monospacedDigit())
            .foregroundStyle(.secondary)
        }
        .padding(.vertical, 4)
        .opacity(usage.isStale(now: now) ? 0.6 : 1)
        .accessibilityElement(children: .combine)
        .accessibilityLabel(
            usage.unlimited
                ? "Copilot monthly premium usage, unlimited, \(usage.resetsLine)"
                : "Copilot monthly premium usage, \(usage.used) of \(usage.limit) \(usage.unit) used, \(percent) percent, \(Int((usage.elapsed(now: now) * 100).rounded())) percent of the month elapsed"
        )
    }
}

private struct SubscriptionUsage: View {
    let usage: PlanUsage
    let now: Date
    @Environment(\.dynamicTypeSize) private var dynamicTypeSize
    @ScaledMetric(relativeTo: .headline) private var logo: CGFloat = 22

    var body: some View {
        let limits = usage.limits(now: now)
        VStack(alignment: .leading, spacing: 8) {
            HStack(spacing: 12) {
                Image("Claude")
                    .resizable()
                    .scaledToFit()
                    .frame(width: logo, height: logo)
                    .padding(logo / 3)
                    .background(.quaternary.opacity(0.6), in: RoundedRectangle(cornerRadius: logo / 2, style: .continuous))
                    .accessibilityHidden(true)
                Text("Claude Code").font(.headline)
            }
            if !limits.isEmpty {
                VStack(alignment: .leading, spacing: 12) {
                    ForEach(limits, id: \.label) { limit in
                        VStack(alignment: .leading, spacing: 8) {
                            Text(limit.title).font(.subheadline)
                            UsageBar(used: limit.used, elapsed: limit.elapsed)
                            let layout = dynamicTypeSize.isAccessibilitySize ? AnyLayout(VStackLayout(alignment: .leading, spacing: 2)) : AnyLayout(HStackLayout())
                            layout {
                                Text("\(limit.used)% used")
                                if !dynamicTypeSize.isAccessibilitySize { Spacer() }
                                Text(limit.resetsLine(now: now))
                            }
                            .font(.caption.monospacedDigit())
                            .foregroundStyle(.secondary)
                        }
                        .accessibilityElement(children: .ignore)
                        .accessibilityLabel(limit.spokenLine(now: now))
                    }
                }
            }
        }
        .padding(.vertical, 4)
        .opacity(usage.isStale(now: now) ? 0.6 : 1)
    }
}

/// Percent used, in the context ring's palette, with a tick where the window's elapsed share is.
private struct UsageBar: View {
    let used: UInt8
    let elapsed: Double

    var body: some View {
        GeometryReader { geo in
            let width = geo.size.width
            ZStack(alignment: .leading) {
                Capsule().fill(.quaternary).frame(height: 4)
                Capsule().fill(ContextRing.tone(100 - used)).frame(width: width * Double(used) / 100, height: 4)
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
