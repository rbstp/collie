import CollieCore
import SwiftUI

/// An agent the machine holds stopped at a Claude Code plan limit, with the time it resumes.
struct LimitBadge: View {
    let limit: PlanLimitSummary
    var compact = false

    var body: some View {
        HStack(spacing: 3) {
            Image(systemName: "hourglass")
            if let text = Self.text(limit, compact: compact) { Text(text).monospacedDigit() }
        }
        .font(.caption2.weight(.medium))
        .foregroundStyle(Self.color)
        .padding(.horizontal, 6)
        .padding(.vertical, 1)
        .background(Self.color.opacity(0.15), in: Capsule())
        .fixedSize()
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(Self.spoken(limit))
    }

    static func text(_ limit: PlanLimitSummary, compact: Bool, now: Date = .now) -> String? {
        guard let at = limit.resumeAtMs else { return compact ? nil : "Limited" }
        return compact ? time(at, now: now) : "Resumes \(time(at, now: now))"
    }

    /// In the phone's time zone; a day other than today is named, as a weekly limit's reset.
    static func time(_ ms: UInt64, now: Date = .now) -> String {
        let date = Date(unixMs: ms)
        if Calendar.current.isDate(date, inSameDayAs: now) {
            return date.formatted(date: .omitted, time: .shortened)
        }
        return date.formatted(.dateTime.weekday(.abbreviated).hour().minute())
    }

    static func spoken(_ limit: PlanLimitSummary, now: Date = .now) -> String {
        let window = switch limit.window {
        case .fiveHour: "5-hour"
        case .sevenDay: "weekly"
        case .unknown: "plan"
        }
        if let at = limit.resumeAtMs { return "Limited at the \(window) limit, resumes at \(time(at, now: now))" }
        return "Limited at the \(window) limit until \(time(limit.resetsAtMs, now: now)), will not resume"
    }

    // System orange is under 3:1 as small text on the pale fill in light mode; this is 4.5:1 or more.
    static let color = Color(uiColor: UIColor {
        $0.userInterfaceStyle == .dark ? .systemOrange : UIColor(red: 0.6, green: 0.3, blue: 0, alpha: 1)
    })
}

/// On the agent screen, the badge opens the limit's actions.
struct LimitMenu: View {
    let limit: PlanLimitSummary
    let cancel: () -> Void
    let continueNow: () -> Void
    @State private var confirming = false

    var body: some View {
        Menu {
            Button("Continue now", systemImage: "play") { confirming = true }
            if limit.resumeAtMs != nil {
                Button("Cancel resume", systemImage: "xmark") { cancel() }
            }
        } label: {
            LimitBadge(limit: limit)
        }
        .buttonStyle(.borderless)
        .fixedSize()
        .accessibilityLabel(LimitBadge.spoken(limit))
        .accessibilityHint("Shows plan limit actions")
        .confirmationDialog("Continue now?", isPresented: $confirming, titleVisibility: .visible) {
            Button("Continue now") { continueNow() }
        } message: {
            Text("The agent runs on extra usage until the limit resets.")
        }
    }
}
