import CollieCore
import Foundation
import Synchronization
import Testing
import UserNotifications

@testable import Collie

@Test func actionIdentifiersMapToDecisions() {
    #expect(ApprovalNotification.decision(forAction: "APPROVE") == .approve)
    #expect(ApprovalNotification.decision(forAction: "DENY") == .deny)
    #expect(ApprovalNotification.decision(forAction: UNNotificationDefaultActionIdentifier) == nil)
    #expect(ApprovalNotification.decision(forAction: UNNotificationDismissActionIdentifier) == nil)
    #expect(ApprovalNotification.decision(forAction: "approve") == nil)

    let info: [AnyHashable: Any] = ["node_id": "nMAC123", "approval_id": "ap_1", "aps": ["category": "APPROVAL"]]
    let link = ApprovalLink(nodeId: "nMAC123", approvalId: "ap_1")!
    #expect(NotificationResponse(actionIdentifier: "APPROVE", userInfo: info) == .decide(link, .approve))
    #expect(NotificationResponse(actionIdentifier: "DENY", userInfo: info) == .decide(link, .deny))
    #expect(NotificationResponse(actionIdentifier: UNNotificationDefaultActionIdentifier, userInfo: info) == .open(link))
    #expect(NotificationResponse(actionIdentifier: UNNotificationDismissActionIdentifier, userInfo: info) == .ignore)
    #expect(NotificationResponse(actionIdentifier: "APPROVE", userInfo: ["approval_id": "ap_1"]) == .ignore)
}

@Test func approvalCategoryNeedsAuthenticationForBothActions() throws {
    let category = try #require(ApprovalNotification.categories.first)
    #expect(ApprovalNotification.categories.count == 1)
    #expect(category.identifier == "APPROVAL")
    #expect(category.actions.map(\.identifier) == ["APPROVE", "DENY"])
    #expect(category.actions.map(\.title) == ["Approve", "Deny"])
    #expect(category.actions.allSatisfy { $0.options.contains(.authenticationRequired) })
    #expect(!category.actions.contains { $0.options.contains(.foreground) })
    #expect(category.actions[1].options.contains(.destructive))
    #expect(!category.actions[0].options.contains(.destructive))
}

@Test func deepLinkParsing() {
    let link = ApprovalLink(userInfo: ["node_id": "nABC123CNTRL", "approval_id": "ap_x-Y_9"])
    #expect(link?.nodeId == "nABC123CNTRL")
    #expect(link?.approvalId == "ap_x-Y_9")
    #expect(link.map { ApprovalLink(userInfo: $0.userInfo) } == link)

    #expect(ApprovalLink(userInfo: [:]) == nil)
    #expect(ApprovalLink(userInfo: ["node_id": "nABC"]) == nil)
    #expect(ApprovalLink(userInfo: ["node_id": "nABC", "approval_id": 7]) == nil)
    #expect(ApprovalLink(userInfo: ["node_id": "", "approval_id": "ap_1"]) == nil)
    #expect(ApprovalLink(userInfo: ["node_id": "nABC", "approval_id": "../ap"]) == nil)
    #expect(ApprovalLink(userInfo: ["node_id": "nABC", "approval_id": "ap 1"]) == nil)
    #expect(ApprovalLink(userInfo: ["node_id": "nABC", "approval_id": "ap_é"]) == nil)
    #expect(ApprovalLink(userInfo: ["node_id": "nABC", "approval_id": String(repeating: "a", count: 65)]) == nil)
    #expect(ApprovalLink(userInfo: ["node_id": "nABC", "approval_id": String(repeating: "a", count: 64)]) != nil)
}

@Test func reachabilityParsing() {
    let json = Data(
        #"{"nUP":{"last_ok_ms":200,"last_fail_ms":100},"nDOWN":{"last_ok_ms":100,"last_fail_ms":200},"nNEVER":{"last_ok_ms":null,"last_fail_ms":5},"nNEW":{"last_ok_ms":7,"last_fail_ms":null}}"#.utf8
    )
    let seen = MacReachability.parse(json)
    #expect(seen.count == 4)
    #expect(seen["nUP"] == MacReachability(lastOkMs: 200, lastFailMs: 100))
    #expect(seen["nUP"]?.lastSeenUnreachable == false)
    #expect(seen["nDOWN"]?.lastSeenUnreachable == true)
    #expect(seen["nNEVER"]?.lastSeenUnreachable == true)
    #expect(seen["nNEW"]?.lastSeenUnreachable == false)
    #expect(MacReachability(lastOkMs: 5, lastFailMs: 5).lastSeenUnreachable == false)
    #expect(MacReachability.parse(Data("{}".utf8)).isEmpty)
    #expect(MacReachability.parse(Data("not json".utf8)).isEmpty)
    #expect(MacReachability.parse(Data(#"{"n":{"last_ok_ms":"x"}}"#.utf8)).isEmpty)
}

@Test func pushBodyRewrite() {
    let hint = Data(#"{"nDOWN":{"last_ok_ms":1,"last_fail_ms":2},"nUP":{"last_ok_ms":2,"last_fail_ms":1}}"#.utf8)
    let body = "Blocked in collie"
    let rewritten = "Blocked in collie (machine may be unreachable, open collie to check)"
    #expect(PushBody.rewrite(body, nodeId: "nDOWN", reachability: hint) == rewritten)
    #expect(PushBody.rewrite(rewritten, nodeId: "nDOWN", reachability: hint) == rewritten)
    #expect(PushBody.rewrite(body, nodeId: "nUP", reachability: hint) == body)
    #expect(PushBody.rewrite(body, nodeId: "nOTHER", reachability: hint) == body)
    #expect(PushBody.rewrite(body, nodeId: nil, reachability: hint) == body)
    #expect(PushBody.rewrite(body, nodeId: "nDOWN", reachability: nil) == body)
    #expect(PushBody.rewrite(body, nodeId: "nDOWN", reachability: Data("garbage".utf8)) == body)
}

@Test func followUpNeverClaimsSuccessWithoutConfirmation() {
    let applied = FollowUp.after(.applied(decision: .approve), decision: .approve, agent: "claude")
    #expect(applied.title == "Approved: claude")
    #expect(!applied.opensApproval)
    #expect(FollowUp.after(.applied(decision: .deny), decision: .deny, agent: "claude").title == "Denied: claude")

    let unconfirmed = FollowUp.after(.unconfirmed(decision: .approve), decision: .approve, agent: "claude")
    #expect(unconfirmed.title.contains("not confirmed"))
    #expect(unconfirmed.opensApproval)

    for outcome: BackgroundOutcome in [
        .unreachable(stage: .connect, message: "timeout"), .unreachable(stage: .nodeUp, message: "x"),
        .unauthorized(message: "x"), .failed(message: "x"),
    ] {
        let followUp = FollowUp.after(outcome, decision: .approve, agent: "claude")
        #expect(followUp.body == "Couldn't reach collied, open to decide")
        #expect(followUp.opensApproval)
    }
    let unknown = FollowUp.after(.unreachable(stage: .decide, message: "x"), decision: .deny, agent: "claude")
    #expect(unknown.body.contains("denial"))
    #expect(unknown.opensApproval)

    let unpaired = FollowUp.after(.unknownMachine, decision: .approve, agent: "claude · omarchy")
    #expect(unpaired.title == "claude · omarchy")
    #expect(unpaired.body == "This machine is no longer paired with this phone. Nothing was sent.")
    #expect(!unpaired.opensApproval)
}

@Test func countdownFormatting() {
    let now = Date(timeIntervalSince1970: 1_700_000_000)
    #expect(Countdown.string(untilMs: 1_700_000_000_000 + 125_000, now: now) == "2:05")
    #expect(Countdown.string(untilMs: 1_700_000_000_000 + 600_000, now: now) == "10:00")
    #expect(Countdown.string(untilMs: 1_700_000_000_000, now: now) == "expired")
    #expect(Countdown.string(untilMs: 0, now: now) == "expired")
}

final class FakeApprovalCore: ApprovalCore {
    struct State {
        var pending: [String: [PendingApproval]] = [:]
        var decisions: [String] = []
        var typed: [String] = []
        var error: CoreError?
        var outcome = DecisionOutcome.applied(decision: .approve, by: "phone")
        var link: [String: LinkPhase] = ["m1": .connected, "m2": .waiting]
        var flocks = 0
    }

    let state = Mutex(State())
    let mac = Machine(id: "m1", label: "Mac", host: "mac.ts.net", port: 8457, nodeId: "nMAC", kind: .mac, key: "")
    let linux = Machine(id: "m2", label: "omarchy", host: "omarchy.ts.net", port: 8457, nodeId: "nLINUX", kind: .linux, key: "")

    func machines() -> [Machine] { [mac, linux] }
    func approvalFeed(machineId: String, afterRevision: UInt64) -> ApprovalFeed? {
        let (link, pending) = state.withLock { ($0.link[machineId] ?? .stopped, $0.pending[machineId] ?? []) }
        return ApprovalFeed(link: link, revision: 1, missed: false, events: [], pending: pending)
    }
    func flock(machineId: String) async throws -> MachineFlock {
        let link = state.withLock { s in
            s.flocks += 1
            return s.link[machineId] ?? .stopped
        }
        let machine = machines().first { $0.id == machineId } ?? mac
        return MachineFlock(machine: machine, link: link, lastError: nil, details: nil, workspaces: [], agents: [], approvalsCount: 0)
    }
    func decide(machineId: String, approvalId: String, decision: ApprovalDecision, note: String?) async throws -> DecisionOutcome {
        let (error, outcome) = state.withLock { s in
            s.decisions.append("\(machineId) \(approvalId) \(decision)" + (note.map { " note=\($0)" } ?? ""))
            return (s.error, s.outcome)
        }
        if let error { throw error }
        return outcome
    }
    func typeText(machineId: String, terminalId: String, text: String) async throws {
        let error = state.withLock { s in
            s.typed.append("\(machineId) \(terminalId) \(text)")
            return s.error
        }
        if let error { throw error }
    }
}

final class FakeAuthenticator: Authenticator {
    struct State {
        var result = true
        var reasons: [String] = []
        var hold = false
        var held: [CheckedContinuation<Void, Never>] = []
    }

    let state = Mutex(State())

    func authenticate(reason: String) async -> Bool {
        let hold = state.withLock { s in
            s.reasons.append(reason)
            return s.hold
        }
        if hold {
            await withCheckedContinuation { continuation in
                state.withLock { $0.held.append(continuation) }
            }
        }
        return state.withLock { $0.result }
    }

    func waitHeld() async {
        while state.withLock({ $0.held.isEmpty }) {
            try? await Task.sleep(for: .milliseconds(5))
        }
    }

    func release(result: Bool) {
        let held = state.withLock { s in
            s.result = result
            s.hold = false
            defer { s.held = [] }
            return s.held
        }
        held.forEach { $0.resume() }
    }
}

private func approval(
    _ id: String, options: [ApprovalDecision] = [.approve, .approveAlways, .deny], choices: [ApprovalChoice] = [],
    acceptsInput: Bool = false, hasTextField: Bool = false, supportsNote: Bool = false
) -> PendingApproval {
    PendingApproval(
        approvalId: id, terminalId: "term_1", agentLabel: "claude", workspaceLabel: "collie", snippet: "Do you want to proceed?",
        toolName: "Bash", toolSummary: "cargo test", options: options, choices: choices, acceptsInput: acceptsInput,
        hasTextField: hasTextField, supportsNote: supportsNote, createdAtMs: 1, expiresAtMs: .max
    )
}

private let planChoices = [
    ApprovalChoice(index: 0, label: "Yes, and use auto mode", current: true),
    ApprovalChoice(index: 1, label: "Yes, manually approve edits", current: false),
    ApprovalChoice(index: 2, label: "Tell Claude what to change", current: false),
]

private let questionChoices = [
    ApprovalChoice(index: 0, label: "PostgreSQL", current: true),
    ApprovalChoice(index: 1, label: "SQLite", current: false),
    ApprovalChoice(index: 2, label: "Type something.", current: false),
]

@MainActor
private func approvalsModel(_ core: FakeApprovalCore, _ auth: FakeAuthenticator) -> ApprovalsModel {
    core.state.withLock { $0.pending["m1"] = [approval("ap_1"), approval("ap_2", options: [.approve, .deny])] }
    let model = ApprovalsModel(core: core, auth: auth)
    model.poll()
    return model
}

@MainActor
@Test func pollKeepsEachMachinesLinkAndDropsExpiredApprovals() async throws {
    let core = FakeApprovalCore()
    var expired = approval("ap_old")
    expired.expiresAtMs = UInt64(Date.now.timeIntervalSince1970 * 1000) - 1
    var linux = approval("ap_linux")
    linux.createdAtMs = 2
    var linuxExpired = approval("ap_linux_old")
    linuxExpired.expiresAtMs = 1
    core.state.withLock { s in
        s.pending["m1"] = [approval("ap_mac"), expired]
        s.pending["m2"] = [linux, linuxExpired]
    }
    let model = ApprovalsModel(core: core, auth: FakeAuthenticator())
    model.poll()
    #expect(model.items.map(\.id) == ["ap_mac", "ap_linux"])
    #expect(model.items.map(\.machine.id) == ["m1", "m2"])
    #expect(model.items.map(\.link) == [.connected, .waiting])
    #expect(model.items.map(\.unreachable) == [false, true])

    let phases: [(LinkPhase, Bool)] = [(.connecting, false), (.connected, false), (.offline, true), (.stopped, true), (.waiting, true), (.unavailable, true)]
    for (link, unreachable) in phases {
        core.state.withLock { $0.link["m2"] = link }
        model.poll()
        #expect(model.items.last?.unreachable == unreachable)
    }

    await model.decide(try #require(model.items.first), .approve)
    #expect(model.notice == "Approved: claude. The agent moved on.")
    core.state.withLock { $0.link["m2"] = .connected }
    model.poll()
    #expect(model.items.last?.link == .connected)
    #expect(model.notice == "Approved: claude. The agent moved on.")
}

@MainActor
@Test func decisionWaitsForLocalAuthentication() async throws {
    let core = FakeApprovalCore()
    let auth = FakeAuthenticator()
    let model = approvalsModel(core, auth)
    let item = try #require(model.items.first)
    #expect(model.items.map(\.id) == ["ap_1", "ap_2"])

    auth.state.withLock { $0.hold = true }
    let decide = Task { await model.decide(item, .approve) }
    await auth.waitHeld()
    #expect(model.steps["ap_1"] == .authenticating(.approve))
    #expect(core.state.withLock { $0.decisions }.isEmpty)

    await model.decide(item, .deny)
    #expect(auth.state.withLock { $0.reasons } == ["Approve claude"])

    auth.release(result: true)
    await decide.value
    #expect(core.state.withLock { $0.decisions } == ["m1 ap_1 approve"])
    #expect(model.steps["ap_1"] == nil)
    #expect(model.notice == "Approved: claude. The agent moved on.")
}

@MainActor
@Test func failedAuthenticationSendsNothing() async throws {
    let core = FakeApprovalCore()
    let auth = FakeAuthenticator()
    auth.state.withLock { $0.result = false }
    let model = approvalsModel(core, auth)
    let item = try #require(model.items.first)

    await model.decide(item, .deny)
    #expect(core.state.withLock { $0.decisions }.isEmpty)
    #expect(model.steps.isEmpty)
    #expect(model.notice?.contains("Nothing was sent") == true)
    #expect(auth.state.withLock { $0.reasons } == ["Deny claude"])

    auth.state.withLock { $0.result = true }
    await model.decide(item, .deny)
    #expect(core.state.withLock { $0.decisions } == ["m1 ap_1 deny"])
}

@MainActor
@Test func decisionsOutsideTheOptionsAreRefused() async throws {
    let core = FakeApprovalCore()
    let auth = FakeAuthenticator()
    let model = approvalsModel(core, auth)
    let item = try #require(model.items.last)
    await model.decide(item, .approveAlways)
    #expect(auth.state.withLock { $0.reasons }.isEmpty)
    #expect(core.state.withLock { $0.decisions }.isEmpty)
}

@MainActor
@Test func unconfirmedAndFailedDecisionsAreReported() async throws {
    let core = FakeApprovalCore()
    let auth = FakeAuthenticator()
    let model = approvalsModel(core, auth)
    let item = try #require(model.items.first)

    core.state.withLock { $0.outcome = .unconfirmed(decision: .approve, by: "phone") }
    await model.decide(item, .approve)
    #expect(model.notice?.contains("not confirmed") == true)

    core.state.withLock { $0.error = .ApprovalExpired }
    await model.decide(item, .approve)
    #expect(model.notice == CoreError.ApprovalExpired.description)
    #expect(model.steps.isEmpty)
}

@MainActor
@Test func agentBannerShowsOnlyThatAgentsApprovals() {
    let core = FakeApprovalCore()
    let model = ApprovalsModel(core: core, auth: FakeAuthenticator())
    var other = approval("ap_9")
    other.terminalId = "term_2"
    core.state.withLock { $0.pending["m1"] = [approval("ap_1"), other] }
    model.poll()
    #expect(model.items(machineId: "m1", terminalId: "term_1").map(\.id) == ["ap_1"])
    #expect(model.items(machineId: "m2", terminalId: "term_1").isEmpty)

    model.open(ApprovalLink(nodeId: "nMAC", approvalId: "ap_9")!)
    #expect(model.highlighted == "ap_9")
}

@MainActor
@Test func notificationTapLoadsUntilTheMacIsConnected() async throws {
    let core = FakeApprovalCore()
    core.state.withLock { $0.link["m1"] = .connecting }
    let model = ApprovalsModel(core: core, auth: FakeAuthenticator())
    let link = try #require(ApprovalLink(nodeId: "nMAC", approvalId: "ap_1"))
    let load = model.open(link)
    #expect(model.loading?.link == link)
    #expect(model.highlighted == "ap_1")
    while core.state.withLock({ $0.flocks }) < 2 {
        try await Task.sleep(for: .milliseconds(5))
    }
    #expect(model.loading?.phase == .connecting)
    #expect(model.items.isEmpty)

    core.state.withLock { s in
        s.link["m1"] = .connected
        s.pending["m1"] = [approval("ap_1")]
    }
    await load?.value
    #expect(model.loading == nil)
    #expect(model.items.map(\.id) == ["ap_1"])
    #expect(model.notice == nil)
}

@MainActor
@Test func notificationTapForAGoneApprovalSaysSoOnceConnected() async throws {
    let core = FakeApprovalCore()
    let model = ApprovalsModel(core: core, auth: FakeAuthenticator())
    let gone = try #require(ApprovalLink(nodeId: "nMAC", approvalId: "ap_gone"))
    await model.open(gone)?.value
    #expect(model.loading == nil)
    #expect(model.notice == "This approval is no longer pending.")

    let unknown = try #require(ApprovalLink(nodeId: "nOTHER", approvalId: "ap_1"))
    #expect(model.open(unknown) == nil)
    #expect(model.loading == nil)
}

@MainActor
@Test func noticeClearsAfterItsLifetime() async throws {
    let core = FakeApprovalCore()
    core.state.withLock { $0.pending["m1"] = [approval("ap_1")] }
    let model = ApprovalsModel(core: core, auth: FakeAuthenticator(), noticeLifetime: .milliseconds(50))
    model.poll()
    let item = try #require(model.items.first)
    core.state.withLock { $0.pending["m1"] = [] }
    await model.decide(item, .approve)
    #expect(model.notice == "Approved: claude. The agent moved on.")
    #expect(model.items.isEmpty)
    while model.notice != nil {
        try await Task.sleep(for: .milliseconds(5))
    }
}

@MainActor
@Test func noticeClearsWhenTheListChangesOtherThanItsApprovalLeaving() async throws {
    let core = FakeApprovalCore()
    let model = approvalsModel(core, FakeAuthenticator())
    let item = try #require(model.items.first)
    await model.decide(item, .approve)
    #expect(model.notice != nil)

    core.state.withLock { $0.pending["m1"] = [approval("ap_2", options: [.approve, .deny])] }
    model.poll()
    #expect(model.notice == "Approved: claude. The agent moved on.")

    core.state.withLock { $0.pending["m1"] = [approval("ap_2", options: [.approve, .deny]), approval("ap_3")] }
    model.poll()
    #expect(model.notice == nil)
}

@MainActor
@Test func headerTapTogglesTheFullPrompt() throws {
    let core = FakeApprovalCore()
    let model = approvalsModel(core, FakeAuthenticator())
    let item = try #require(model.items.first)
    #expect(model.expanded.isEmpty)
    model.toggleExpanded(item)
    #expect(model.expanded == ["ap_1"])
    model.toggleExpanded(item)
    #expect(model.expanded.isEmpty)

    model.toggleExpanded(item)
    core.state.withLock { $0.pending["m1"] = [] }
    model.poll()
    #expect(model.expanded.isEmpty)
}

@MainActor
@Test func menuOptionIsChosenOnlyAfterAuthenticationAndOnlyWithoutDecisions() async throws {
    let core = FakeApprovalCore()
    let auth = FakeAuthenticator()
    core.state.withLock { s in
        s.pending["m1"] = [approval("ap_q", options: [], choices: questionChoices), approval("ap_b", choices: questionChoices)]
        s.outcome = .applied(decision: .choose(choice: 1), by: "phone")
    }
    let model = ApprovalsModel(core: core, auth: auth)
    model.poll()
    let question = try #require(model.items.first { $0.id == "ap_q" })
    let bash = try #require(model.items.first { $0.id == "ap_b" })

    await model.decide(bash, .choose(choice: 0))
    await model.decide(question, .choose(choice: 3))
    await model.decide(question, .approve)
    #expect(auth.state.withLock { $0.reasons }.isEmpty)
    #expect(core.state.withLock { $0.decisions }.isEmpty)

    auth.state.withLock { $0.result = false }
    await model.decide(question, .choose(choice: 1))
    #expect(core.state.withLock { $0.decisions }.isEmpty)
    #expect(model.notice?.contains("Nothing was sent") == true)

    auth.state.withLock { $0.result = true }
    await model.decide(question, .choose(choice: 1))
    #expect(auth.state.withLock { $0.reasons } == ["Choose option 2 for claude", "Choose option 2 for claude"])
    #expect(core.state.withLock { $0.decisions } == ["m1 ap_q choose(choice: 1)"])
    #expect(model.notice == "Chose option 2: claude. The agent moved on.")

    core.state.withLock { $0.outcome = .superseded }
    await model.decide(question, .choose(choice: 0))
    #expect(model.notice == "The prompt changed on the machine. Nothing was sent.")
}

@MainActor
@Test func keysAndTextAreOfferedOnlyWhereColliedAcceptsInput() {
    let core = FakeApprovalCore()
    let model = ApprovalsModel(core: core, auth: FakeAuthenticator())
    #expect(model.blockedInput(machineId: "m1", terminalId: "term_1") == nil)

    core.state.withLock { $0.pending["m1"] = [approval("ap_b", choices: questionChoices)] }
    model.poll()
    #expect(model.blockedInput(machineId: "m1", terminalId: "term_1") == .optionsOnly)

    core.state.withLock { $0.pending["m1"] = [approval("ap_plan", options: [], choices: questionChoices)] }
    model.poll()
    #expect(model.blockedInput(machineId: "m1", terminalId: "term_1") == .optionsOnly)

    core.state.withLock {
        $0.pending["m1"] = [approval("ap_q", options: [], choices: questionChoices, acceptsInput: true, hasTextField: true)]
    }
    model.poll()
    #expect(model.blockedInput(machineId: "m1", terminalId: "term_1") == .keysAndText)
    #expect(model.blockedInput(machineId: "m1", terminalId: "term_2") == nil)

    core.state.withLock { $0.pending["m1"] = [approval("ap_k", options: [], choices: questionChoices, acceptsInput: true)] }
    model.poll()
    #expect(model.blockedInput(machineId: "m1", terminalId: "term_1") == .keys)
}

@MainActor
@Test func noteGoesWithApproveOrDenyOnlyAfterAuthentication() async throws {
    let core = FakeApprovalCore()
    let auth = FakeAuthenticator()
    core.state.withLock { $0.pending["m1"] = [approval("ap_n", supportsNote: true), approval("ap_old")] }
    let model = ApprovalsModel(core: core, auth: auth)
    model.poll()
    let item = try #require(model.items.first { $0.id == "ap_n" })
    let old = try #require(model.items.first { $0.id == "ap_old" })

    model.drafts[item.id] = "use a .tmp extension"
    #expect(model.note(for: item) == nil)
    model.toggleNote(item)
    #expect(model.note(for: item) == "use a .tmp extension")

    await model.decide(item, .approveAlways)
    #expect(auth.state.withLock { $0.reasons }.isEmpty)
    #expect(core.state.withLock { $0.decisions }.isEmpty)

    auth.state.withLock { $0.result = false }
    await model.decide(item, .approve)
    #expect(core.state.withLock { $0.decisions }.isEmpty)
    #expect(model.drafts[item.id] == "use a .tmp extension")

    auth.state.withLock { $0.result = true }
    model.drafts[item.id] = "  use a .tmp extension \n"
    await model.decide(item, .approve)
    core.state.withLock { $0.outcome = .applied(decision: .deny, by: "phone") }
    await model.decide(item, .deny)
    #expect(core.state.withLock { $0.decisions } == ["m1 ap_n approve note=use a .tmp extension", "m1 ap_n deny note=use a .tmp extension"])
    #expect(model.notice == "Denied: claude. The agent moved on.")

    model.drafts[item.id] = "   "
    await model.decide(item, .approveAlways)
    model.toggleNote(item)
    model.drafts[item.id] = "hidden"
    await model.decide(item, .approve)
    #expect(core.state.withLock { $0.decisions }.suffix(2) == ["m1 ap_n approveAlways", "m1 ap_n approve"])

    model.drafts[old.id] = "no amend"
    model.toggleNote(old)
    await model.decide(old, .approve)
    #expect(core.state.withLock { $0.decisions }.count == 4)

    core.state.withLock { $0.pending["m1"] = [] }
    model.poll()
    #expect(model.drafts.isEmpty && model.noting.isEmpty)
}

@MainActor
@Test func planFeedbackIsTypedIntoItsTextField() async throws {
    let core = FakeApprovalCore()
    let auth = FakeAuthenticator()
    core.state.withLock {
        $0.pending["m1"] = [
            approval("ap_plan", options: [], choices: planChoices, hasTextField: true),
            approval("ap_q", options: [], choices: questionChoices, acceptsInput: true, hasTextField: true),
        ]
    }
    let model = ApprovalsModel(core: core, auth: auth)
    model.poll()
    let plan = try #require(model.items.first { $0.id == "ap_plan" })
    let question = try #require(model.items.first { $0.id == "ap_q" })
    #expect(plan.approval.takesFeedback && !question.approval.takesFeedback)
    #expect(model.blockedInput(machineId: "m1", terminalId: "term_1") == .optionsOnly)

    await model.sendFeedback(plan)
    model.drafts[question.id] = "MySQL"
    await model.sendFeedback(question)
    #expect(core.state.withLock { $0.typed }.isEmpty)

    model.drafts[plan.id] = "  use echo instead \n"
    core.state.withLock { $0.error = .AgentBlocked }
    await model.sendFeedback(plan)
    #expect(model.drafts[plan.id] == "  use echo instead \n")
    #expect(model.steps.isEmpty)

    core.state.withLock { $0.error = nil }
    await model.sendFeedback(plan)
    #expect(core.state.withLock { $0.typed } == ["m1 term_1 use echo instead", "m1 term_1 use echo instead"])
    #expect(model.drafts[plan.id] == nil)
    #expect(model.notice == "Sent your feedback to claude.")
    #expect(auth.state.withLock { $0.reasons }.isEmpty)
    #expect(core.state.withLock { $0.decisions }.isEmpty)
}

@MainActor
@Test func answeredApprovalsAreDismissedFromNotificationCenter() {
    let core = FakeApprovalCore()
    core.state.withLock { s in
        s.pending["m1"] = [approval("ap_1"), approval("ap_2")]
        s.pending["m2"] = [approval("ap_linux")]
    }
    let swept = Mutex<[String]>([])
    let model = ApprovalsModel(core: core, auth: FakeAuthenticator()) { nodeId, pending in
        swept.withLock { $0.append("\(nodeId) \(pending.sorted())") }
    }
    model.poll()
    model.poll()
    #expect(swept.withLock { $0 } == ["nMAC [\"ap_1\", \"ap_2\"]"], "once per change, connected machines only")

    core.state.withLock { $0.pending["m1"] = [approval("ap_2")] }
    model.poll()
    #expect(swept.withLock { $0.last } == "nMAC [\"ap_2\"]", "answered in the app or on the machine")

    core.state.withLock { $0.link["m1"] = .waiting }
    model.poll()
    core.state.withLock { $0.link["m1"] = .connected }
    model.poll()
    #expect(swept.withLock { $0.count } == 3, "swept again after a reconnect, for what was answered meanwhile")
}
