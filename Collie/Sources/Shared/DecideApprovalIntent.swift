import AppIntents

// Compiled into the app and CollieWidgets, so iOS runs `perform` in the app process: each
// target supplies its own `perform`, and only the app's decides.

/// Approve or Deny on a followed agent's Live Activity.
struct DecideApprovalIntent: LiveActivityIntent {
    static let title: LocalizedStringResource = "Decide approval"
    static let isDiscoverable = false
    static let openAppWhenRun = false
    /// Like the notification actions' `.authenticationRequired`: the device must be unlocked.
    static let authenticationPolicy = IntentAuthenticationPolicy.requiresAuthentication

    @Parameter(title: "Mac node id")
    var nodeId: String

    @Parameter(title: "Approval id")
    var approvalId: String

    @Parameter(title: "Decision")
    var decision: ActivityDecision

    init() {}

    init(nodeId: String, approvalId: String, decision: ActivityDecision) {
        self.nodeId = nodeId
        self.approvalId = approvalId
        self.decision = decision
    }
}

enum ActivityDecision: String, AppEnum {
    case approve, deny

    static let typeDisplayRepresentation: TypeDisplayRepresentation = "Decision"
    static let caseDisplayRepresentations: [ActivityDecision: DisplayRepresentation] = [.approve: "Approve", .deny: "Deny"]
}
