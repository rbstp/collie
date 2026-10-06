import CollieCore
import SwiftUI

/// The Agents tab's Usage view: one card per machine with the plan usage collied last recorded.
struct UsageSections: View {
    let entries: [MachineFlockEntry]
    @Environment(\.dynamicTypeSize) private var dynamicTypeSize

    var body: some View {
        let recorded = entries.compactMap { entry in entry.flock?.planUsage.map { (entry, $0) } }
        if recorded.isEmpty {
            Text("No plan usage recorded").foregroundStyle(.secondary)
        }
        ForEach(recorded, id: \.0.id) { entry, usage in
            Section {
                TimelineView(.everyMinute) { context in
                    let layout = dynamicTypeSize.isAccessibilitySize ? AnyLayout(VStackLayout(alignment: .leading, spacing: 2)) : AnyLayout(HStackLayout())
                    layout {
                        MachineName(machine: entry.machine).font(.subheadline.weight(.semibold)).foregroundStyle(.primary)
                        if !dynamicTypeSize.isAccessibilitySize { Spacer() }
                        Text(usage.age(now: context.date)).font(.caption).foregroundStyle(.secondary)
                    }
                    .accessibilityElement(children: .combine)
                    .accessibilityLabel("\(entry.machine.label), \(usage.spokenAge(now: context.date))")
                }
                TimelineView(.everyMinute) { context in
                    SubscriptionUsage(usage: usage, now: context.date)
                }
            }
            .opacity(entry.linkDown ? 0.5 : 1)
        }
    }
}

private struct SubscriptionUsage: View {
    let usage: PlanUsage
    let now: Date
    @Environment(\.dynamicTypeSize) private var dynamicTypeSize
    @ScaledMetric(relativeTo: .headline) private var logo: CGFloat = 22

    var body: some View {
        let limits = usage.limits(now: now)
        VStack(alignment: .leading, spacing: 10) {
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
                Group {
                    if dynamicTypeSize.isAccessibilitySize {
                        VStack(alignment: .leading, spacing: 10) {
                            ForEach(limits, id: \.label) { limit in
                                VStack(alignment: .leading, spacing: 4) {
                                    HStack {
                                        Text(limit.label).foregroundStyle(.secondary)
                                        Spacer()
                                        Text("\(limit.used)%")
                                    }
                                    Text(limit.resets(now: now)).foregroundStyle(.secondary)
                                    UsageBar(limit: limit)
                                }
                            }
                        }
                    } else {
                        Grid(alignment: .leading, horizontalSpacing: 12, verticalSpacing: 8) {
                            ForEach(limits, id: \.label) { limit in
                                GridRow {
                                    Text(limit.label).foregroundStyle(.secondary)
                                    UsageBar(limit: limit)
                                    Text("\(limit.used)%").gridColumnAlignment(.trailing)
                                    Text(limit.resets(now: now)).foregroundStyle(.secondary).gridColumnAlignment(.trailing)
                                }
                            }
                        }
                        .lineLimit(1)
                    }
                }
                .font(.subheadline.monospacedDigit())
                .accessibilityElement(children: .ignore)
                .accessibilityLabel(limits.map { $0.spokenLine(now: now) }.joined(separator: ". "))
                Text(limits.map(\.paceText).joined(separator: " · "))
                    .font(.footnote)
                    .foregroundStyle(.secondary)
                    .accessibilityLabel(limits.map(\.spokenPace).joined(separator: ", "))
            }
        }
        .padding(.vertical, 4)
        .opacity(usage.isStale(now: now) ? 0.6 : 1)
    }
}

/// Percent used, in the context ring's palette, with a tick where the window's elapsed share is.
private struct UsageBar: View {
    let limit: UsageLimit

    var body: some View {
        GeometryReader { geo in
            let width = geo.size.width
            ZStack(alignment: .leading) {
                Capsule().fill(.quaternary).frame(height: 4)
                Capsule().fill(ContextRing.tone(limit.left)).frame(width: width * Double(limit.used) / 100, height: 4)
                Capsule()
                    .fill(.secondary)
                    .frame(width: 2, height: 12)
                    .offset(x: min(max(width * limit.elapsed - 1, 0), width - 2))
            }
            .frame(maxHeight: .infinity)
        }
        .frame(minWidth: 60, maxWidth: .infinity)
        .frame(height: 12)
    }
}
