import CollieCore
import Foundation
import UserNotifications

/// Points at one approval on one Mac. Comes from a push payload, which only Apple and collied
/// can send, but is still validated: it is a lookup key, never trusted for anything else.
struct ApprovalLink: Hashable, Sendable {
    let nodeId: String
    let approvalId: String

    init?(nodeId: String, approvalId: String) {
        guard ProtocolId.valid(nodeId, extra: "-_"), ProtocolId.valid(approvalId, extra: "-_") else { return nil }
        self.nodeId = nodeId
        self.approvalId = approvalId
    }

    init?(userInfo: [AnyHashable: Any]) {
        guard let nodeId = userInfo["node_id"] as? String, let approvalId = userInfo["approval_id"] as? String else {
            return nil
        }
        self.init(nodeId: nodeId, approvalId: approvalId)
    }

    var userInfo: [String: String] { ["node_id": nodeId, "approval_id": approvalId] }
}

/// collied's background push once an alerted approval is over: it only removes that
/// approval's delivered alert, never decides or reaches the Mac.
enum ApprovalClear {
    /// Nil for anything with an alert: an approval alert can reach the app too while it runs.
    static func link(_ userInfo: [AnyHashable: Any]) -> ApprovalLink? {
        guard let aps = userInfo["aps"] as? [String: Any], aps["alert"] == nil, aps["content-available"] as? Int == 1 else {
            return nil
        }
        return ApprovalLink(userInfo: userInfo)
    }

    static func identifiers(_ delivered: [(id: String, userInfo: [AnyHashable: Any])], naming link: ApprovalLink) -> [String] {
        delivered.filter { ApprovalLink(userInfo: $0.userInfo) == link }.map(\.id)
    }

    static func remove(_ link: ApprovalLink) async {
        let center = UNUserNotificationCenter.current()
        let delivered = await center.deliveredNotifications().map { (id: $0.request.identifier, userInfo: $0.request.content.userInfo) }
        center.removeDeliveredNotifications(withIdentifiers: identifiers(delivered, naming: link))
    }
}

enum ApprovalNotification {
    static let category = "APPROVAL"
    static let approveAction = "APPROVE"
    static let denyAction = "DENY"

    /// Both actions need the device unlocked: they run in the background without opening the app.
    static var categories: Set<UNNotificationCategory> {
        [
            UNNotificationCategory(
                identifier: category,
                actions: [
                    UNNotificationAction(identifier: approveAction, title: "Approve", options: [.authenticationRequired]),
                    UNNotificationAction(identifier: denyAction, title: "Deny", options: [.authenticationRequired, .destructive]),
                ],
                intentIdentifiers: []
            )
        ]
    }

    static func decision(forAction identifier: String) -> ApprovalDecision? {
        switch identifier {
        case approveAction: .approve
        case denyAction: .deny
        default: nil
        }
    }
}

enum NotificationResponse: Equatable, Sendable {
    case decide(ApprovalLink, ApprovalDecision)
    case open(ApprovalLink)
    case ignore

    init(actionIdentifier: String, userInfo: [AnyHashable: Any]) {
        guard let link = ApprovalLink(userInfo: userInfo) else {
            self = .ignore
            return
        }
        if let decision = ApprovalNotification.decision(forAction: actionIdentifier) {
            self = .decide(link, decision)
        } else if actionIdentifier == UNNotificationDefaultActionIdentifier {
            self = .open(link)
        } else {
            self = .ignore
        }
    }
}

/// The local notification posted after a lock-screen decision. Never claims success unless
/// collied saw the agent leave `blocked`.
struct FollowUp: Equatable {
    let title: String
    let body: String
    let opensApproval: Bool

    static let unreachable = "Couldn't reach collied, open to decide"

    static func after(_ outcome: BackgroundOutcome, decision: ApprovalDecision, agent: String) -> FollowUp {
        switch outcome {
        case .applied(let decision):
            FollowUp(title: "\(decision.pastTense): \(agent)", body: "The agent moved on.", opensApproval: false)
        case .unconfirmed(let decision):
            FollowUp(
                title: "\(decision.pastTense), not confirmed: \(agent)",
                body: "The keys were sent but the agent still looks blocked. Open collie to check.",
                opensApproval: true
            )
        case .expired:
            FollowUp(title: agent, body: "This approval expired. Nothing was sent.", opensApproval: false)
        case .superseded:
            FollowUp(title: agent, body: "The prompt changed on the machine. Nothing was sent; open collie to see it.", opensApproval: true)
        case .alreadyResolved, .notFound:
            FollowUp(title: agent, body: "This approval is no longer pending. Nothing was sent.", opensApproval: false)
        case .unreachable(stage: .decide, message: _):
            FollowUp(
                title: agent,
                body: "Couldn't confirm the \(decision.noun) reached collied. Open collie to check.",
                opensApproval: true
            )
        case .unknownMachine:
            FollowUp(title: agent, body: "This machine is no longer paired with this phone. Nothing was sent.", opensApproval: false)
        case .unreachable, .unauthorized, .failed:
            FollowUp(title: agent, body: unreachable, opensApproval: true)
        }
    }
}

extension ApprovalDecision {
    var title: String {
        switch self {
        case .approve: "Approve"
        case .approveAlways: "Approve always"
        case .deny: "Deny"
        case .choose(let choice): "Choose option \(Int(choice) + 1)"
        }
    }

    var pastTense: String {
        switch self {
        case .approve: "Approved"
        case .approveAlways: "Always approved"
        case .deny: "Denied"
        case .choose(let choice): "Chose option \(Int(choice) + 1)"
        }
    }

    var noun: String {
        switch self {
        case .deny: "denial"
        case .choose: "choice"
        case .approve, .approveAlways: "approval"
        }
    }

    func reason(agent: String) -> String {
        if case .choose = self { return "\(title) for \(agent)" }
        return "\(title) \(agent)"
    }

    var progressive: String {
        switch self {
        case .approve: "Approving…"
        case .approveAlways: "Always approving…"
        case .deny: "Denying…"
        case .choose: "Choosing…"
        }
    }
}

extension PushEnvironment {
    /// `CollieApsEnvironment` and the `aps-environment` entitlement come from the same build setting.
    static var current: PushEnvironment {
        Bundle.main.object(forInfoDictionaryKey: "CollieApsEnvironment") as? String == "production" ? .production : .sandbox
    }
}
