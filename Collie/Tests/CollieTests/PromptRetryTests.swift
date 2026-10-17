import CollieCore
import Foundation
import Synchronization
import Testing

@testable import Collie

private let unreachable = CoreError.Unreachable(message: "timed out waiting for the machine")
private let notSent = "May not have been sent: machine unreachable"

private final class TestClock: Sendable {
    private let state = Mutex((now: ContinuousClock.now, sleeps: [Duration]()))
    let start: ContinuousClock.Instant

    init() { start = state.withLock { $0.now } }

    var now: ContinuousClock.Instant { state.withLock { $0.now } }
    var sleeps: [Duration] { state.withLock { $0.sleeps } }

    func sleep(_ duration: Duration) {
        state.withLock {
            $0.now += duration
            $0.sleeps.append(duration)
        }
    }
}

@MainActor
private func retryingModel(
    _ core: FakeCore, _ clock: TestClock, resends: PromptResends = PromptResends(), draftsFile: URL? = nil,
    sleep: @escaping @Sendable (Duration) async throws -> Void
) async -> AgentModel {
    core.state.withLock {
        $0.kind = "claude"
        $0.macDraft = ""
    }
    let model = AgentModel(
        core: core, route: route("term_1"), prefsFile: nil, draftsFile: draftsFile, now: { clock.now }, sleep: sleep, resends: resends
    )
    model.poll()
    await model.loadMacDraft()
    return model
}

@Test func onlyAnUnreachableMachineIsRetried() {
    #expect(PromptRetry.retries(unreachable))
    #expect(PromptRetry.retries(CoreError.NotRunning))
    let refusals: [CoreError] = [
        .AgentBlocked(message: ""), .AgentNotReady, .DraftChanged(current: "x"), .DraftNotCleared,
        .InvalidInput(field: "text", message: "too long"), .RateLimited, .NotFound, .HerdrUnavailable,
        .Rejected(message: ""), .Internal(message: "unexpected reply from the machine: flock"),
        .Unauthorized(message: ""),
    ]
    for error in refusals {
        #expect(!PromptRetry.retries(error), "\(error)")
    }
    #expect(!PromptRetry.retries(CancellationError()))
}

@Test func fiveAttemptsAMinuteApart() throws {
    let start = ContinuousClock.now
    var retry = PromptRetry(started: start)
    #expect(retry.next(after: CoreError.AgentNotReady) == nil)
    var began: [Bool] = []
    for attempt in 1..<PromptRetry.attempts {
        let at = try #require(retry.next(after: unreachable))
        #expect(at == start + .seconds(60 * attempt))
        began.append(retry.begin(at: at))
    }
    #expect(began == [true, true, true, true])
    #expect(retry.attempt == 5)
    #expect(retry.next(after: unreachable) == nil)
    let sixth = retry.begin(at: start + .seconds(250))
    #expect(!sixth)
}

@Test func anEarlyAttemptMovesTheNextOne() {
    let start = ContinuousClock.now
    var retry = PromptRetry(started: start)
    let began = retry.begin(at: start + .seconds(10))
    #expect(began)
    #expect(retry.next(after: unreachable) == start + .seconds(70))
}

/// Past this, the core may have forgotten the op_id, so a resend could type the prompt twice.
@Test func noAttemptStartsAfterTheWindow() {
    let start = ContinuousClock.now
    var retry = PromptRetry(started: start)
    let late = retry.begin(at: start + .seconds(271))
    #expect(!late)
    #expect(retry.attempt == 1)
    let inTime = retry.begin(at: start + .seconds(270))
    #expect(inTime)
}

@MainActor
@Test func anUnreachableMachineGetsTheSamePromptFiveTimesThenTheError() async {
    let core = FakeCore()
    core.set(error: unreachable)
    let clock = TestClock()
    let model = await retryingModel(core, clock) { clock.sleep($0) }
    model.draft = "fix the build"
    await model.sendPrompt()
    #expect(core.snapshot.prompts == Array(repeating: "fix the build", count: 5))
    #expect(core.snapshot.expectedDrafts == Array(repeating: "", count: 5))
    #expect(core.snapshot.resends == Array(repeating: "fix the build", count: 4), "only the tap mints an op_id")
    #expect(clock.sleeps == Array(repeating: .seconds(60), count: 4))
    #expect(model.promptError == notSent)
    #expect(model.draft == "fix the build")
    #expect(!model.retrying)
    #expect(!model.sendingPrompt)
}

@MainActor
@Test func aResendThatArrivesClearsTheField() async {
    let core = FakeCore()
    core.set(error: .NotRunning)
    let clock = TestClock()
    let model = await retryingModel(core, clock) { [core] in
        clock.sleep($0)
        core.set(error: nil)
    }
    model.draft = "fix the build"
    await model.sendPrompt()
    #expect(core.snapshot.prompts == ["fix the build", "fix the build"])
    #expect(model.promptError == nil)
    #expect(model.draft.isEmpty)
}

@MainActor
@Test func aRefusalEndsTheRetries() async {
    let core = FakeCore()
    core.set(error: unreachable)
    let clock = TestClock()
    let model = await retryingModel(core, clock) { [core] in
        clock.sleep($0)
        core.set(error: .AgentBlocked(message: ""))
    }
    model.draft = "continue"
    await model.sendPrompt()
    #expect(core.snapshot.prompts == ["continue", "continue"])
    #expect(model.promptError?.contains("approval") == true)
    #expect(model.draft == "continue")

    core.set(error: .DraftChanged(current: "typed on the Mac"))
    await model.sendPrompt()
    #expect(core.snapshot.prompts.count == 3)
    #expect(clock.sleeps.count == 1)
}

@MainActor
private func waitingModel(
    _ core: FakeCore, resends: PromptResends = PromptResends(), draftsFile: URL? = nil
) async -> (AgentModel, Task<Void, Never>) {
    core.set(error: unreachable)
    let model = await retryingModel(core, TestClock(), resends: resends, draftsFile: draftsFile) { try await Task.sleep(for: $0) }
    model.draft = "fix the build"
    let send = Task { await model.sendPrompt() }
    await poll { model.retrying }
    return (model, send)
}

@MainActor
@Test func stoppingTheRetryKeepsThePrompt() async {
    let core = FakeCore()
    let (model, send) = await waitingModel(core)
    #expect(model.sendingPrompt)
    model.stopRetry()
    await send.value
    #expect(core.snapshot.prompts == ["fix the build"])
    #expect(model.draft == "fix the build")
    #expect(!model.retrying)
    #expect(model.promptError == notSent)
}

@MainActor
@Test func editingThePromptStopsTheRetry() async {
    let core = FakeCore()
    let (model, send) = await waitingModel(core)
    model.typed("fix the build and the tests")
    await send.value
    #expect(core.snapshot.prompts == ["fix the build"])
    #expect(model.draft == "fix the build and the tests")
    #expect(!model.retrying)
}

@MainActor
@Test func theMachineComingBackSendsAtOnce() async {
    let core = FakeCore()
    core.state.withLock { $0.link = .connecting }
    let (model, send) = await waitingModel(core)
    core.set(error: nil)
    core.state.withLock { $0.link = .connected }
    model.poll()
    await send.value
    #expect(core.snapshot.prompts == ["fix the build", "fix the build"])
    #expect(model.draft.isEmpty)
    #expect(model.promptError == nil)
}

@Test func aChangedLastPromptMeansTheResendArrived() {
    #expect(PromptRetry.arrived("  fix the build\nand the tests", lastPrompt: "fix the build", before: "older"))
    #expect(!PromptRetry.arrived("continue", lastPrompt: "continue", before: nil), "unknown at the start")
    #expect(!PromptRetry.arrived("fix the build", lastPrompt: "fix the build", before: "fix the build"))
    #expect(!PromptRetry.arrived("fix the build", lastPrompt: nil, before: "older"))
    #expect(!PromptRetry.arrived("fix the build", lastPrompt: "fix the tests", before: "older"))
    let long = String(repeating: "a", count: 200)
    #expect(PromptRetry.arrived(long, lastPrompt: String(repeating: "a", count: 159) + "…", before: "older"))
}

@MainActor
@Test func theMachineBackDuringAnAttemptSendsAgainAtOnce() async {
    let core = FakeCore()
    core.state.withLock { $0.link = .connecting }
    let clock = TestClock()
    let model = await retryingModel(core, clock) { clock.sleep($0) }
    model.draft = "fix the build"
    core.set(hold: true, error: unreachable)
    let send = Task { await model.sendPrompt() }
    await core.waitHeld(1)
    core.state.withLock { $0.link = .connected }
    model.poll()
    core.release()
    await send.value
    #expect(core.snapshot.prompts.count == 5)
    #expect(clock.sleeps.count == 3, "no wait after the first attempt")
}

@MainActor
@Test func aSuspensionPastTheWindowStopsTheRetry() async {
    let core = FakeCore()
    core.set(error: unreachable)
    let clock = TestClock()
    let model = await retryingModel(core, clock) { clock.sleep($0 + .seconds(240)) }
    model.draft = "fix the build"
    await model.sendPrompt()
    #expect(core.snapshot.prompts == ["fix the build"])
    #expect(model.promptError == notSent)
    #expect(model.draft == "fix the build")
}

/// A second Send of the same text reuses the first series' op_id; once the core lets it go,
/// the retries stop instead of sending under a new one.
@MainActor
@Test func aSecondSeriesNeverMintsAnOpIdForItsRetries() async {
    let core = FakeCore()
    core.state.withLock { $0.link = .connecting }
    let (model, first) = await waitingModel(core)
    model.stopRetry()
    await first.value
    let second = Task { await model.sendPrompt() }
    await poll { model.retrying }
    core.state.withLock { $0.heldOps.removeAll() }
    core.state.withLock { $0.link = .connected }
    model.poll()
    await second.value
    #expect(core.snapshot.prompts == ["fix the build", "fix the build"])
    #expect(core.snapshot.resends.isEmpty)
    #expect(model.promptError == CoreError.NotResent.description)
    #expect(model.draft == "fix the build")
}

@MainActor
@Test func aPromptTheAgentAlreadyShowsIsNotResent() async {
    let core = FakeCore()
    core.state.withLock { $0.lastPrompt = "run the tests" }
    let resends = PromptResends()
    let (model, send) = await waitingModel(core, resends: resends)
    core.state.withLock { $0.lastPrompt = "fix the build" }
    resends.wake(route("term_1"))
    await send.value
    #expect(core.snapshot.prompts == ["fix the build"])
    #expect(core.snapshot.resends.isEmpty)
    #expect(model.draft.isEmpty)
    #expect(model.promptError == nil)
}

/// While the link is down the agent's last prompt can be stale, so it is not compared.
@MainActor
@Test func aLastPromptSeenWhileUnlinkedIsNotCompared() async {
    let core = FakeCore()
    core.state.withLock {
        $0.lastPrompt = "run the tests"
        $0.link = .connecting
    }
    let resends = PromptResends()
    let (_, send) = await waitingModel(core, resends: resends)
    core.set(error: nil)
    core.state.withLock { $0.lastPrompt = "fix the build" }
    resends.wake(route("term_1"))
    await send.value
    #expect(core.snapshot.resends == ["fix the build"])
}

@MainActor
@Test func anAgentExitingMidRetryLeavesNoError() async {
    let core = FakeCore()
    let (model, send) = await waitingModel(core)
    core.state.withLock {
        $0.kind = nil
        $0.shell = TerminalSummary(terminalId: "term_1", workspaceId: "w1", workspaceLabel: "collie", label: nil, cwd: nil, locked: true)
    }
    model.poll()
    await send.value
    #expect(model.isTerminal)
    #expect(model.promptError == nil)
    #expect(model.draft.isEmpty)
}

@MainActor
@Test func anUnknownLastPromptIsNeverTakenAsArrived() async {
    let core = FakeCore()
    core.state.withLock { $0.link = .connecting }
    let (model, send) = await waitingModel(core)
    core.set(error: nil)
    core.state.withLock {
        $0.lastPrompt = "fix the build"
        $0.link = .connected
    }
    model.poll()
    await send.value
    #expect(core.snapshot.resends == ["fix the build"], "sent again, under the held op_id")
    #expect(model.draft.isEmpty)
}

@MainActor
private func leave(_ model: AgentModel, _ core: FakeCore) async {
    let watches = core.snapshot.watches.filter { $0 == model.route.terminalId }.count
    let run = Task { await model.run() }
    await poll { core.snapshot.watches.filter { $0 == model.route.terminalId }.count > watches }
    run.cancel()
    await run.value
}

@MainActor
@Test func leavingTheScreenKeepsResendingUntilItArrives() async throws {
    let core = FakeCore()
    let drafts = try makeTemporaryDirectory().appending(path: "drafts.json")
    let resends = PromptResends()
    let (model, send) = await waitingModel(core, resends: resends, draftsFile: drafts)
    await leave(model, core)
    core.set(error: nil)
    resends.wake(route("term_1"))
    await send.value
    #expect(core.snapshot.resends == ["fix the build"])
    #expect(resends.entries.isEmpty)
    #expect(AgentDrafts.load(from: drafts).drafts.isEmpty, "the delivered prompt is not kept as a draft")
}

@MainActor
@Test func leavingDuringTheFirstAttemptStillResends() async {
    let core = FakeCore()
    let resends = PromptResends()
    let clock = TestClock()
    let model = await retryingModel(core, clock, resends: resends) { [core] in
        clock.sleep($0)
        core.set(error: nil)
    }
    model.draft = "fix the build"
    core.set(hold: true, error: unreachable)
    let send = Task { await model.sendPrompt() }
    await core.waitHeld(1)
    await leave(model, core)
    core.release()
    await send.value
    #expect(core.snapshot.prompts == ["fix the build", "fix the build"])
    #expect(core.snapshot.resends == ["fix the build"])
}

@MainActor
@Test func aReopenedScreenShowsThePendingPrompt() async {
    let core = FakeCore()
    let resends = PromptResends()
    let (first, send) = await waitingModel(core, resends: resends)
    await leave(first, core)
    core.state.withLock { $0.link = .connecting }
    let second = await retryingModel(core, TestClock(), resends: resends) { _ in }
    #expect(second.draft == "fix the build")
    #expect(second.sendingPrompt)
    #expect(second.retrying)

    core.set(error: nil)
    core.state.withLock { $0.link = .connected }
    second.poll()
    await send.value
    await poll { !second.sendingPrompt }
    #expect(second.draft.isEmpty)
    #expect(second.promptError == nil)
    #expect(core.snapshot.resends == ["fix the build"])
}

@MainActor
@Test func newTextFromAReopenedScreenStopsThePendingPromptFirst() async {
    let core = FakeCore()
    let resends = PromptResends()
    let (first, send) = await waitingModel(core, resends: resends)
    await leave(first, core)
    core.set(error: nil)
    let second = AgentModel(core: core, route: route("term_1"), prefsFile: nil, resends: resends)
    second.draft = "something else"
    second.poll()
    #expect(!second.retrying, "a field with text does not take the pending prompt")
    await second.sendPrompt()
    await send.value
    #expect(core.snapshot.prompts == ["fix the build", "something else"])
    #expect(core.snapshot.resends.isEmpty)
    #expect(resends.entries.isEmpty)
    #expect(second.promptError == nil)
}

@MainActor
@Test func aFinalFailureWithNoScreenRestoresTheDraftOnce() async throws {
    let core = FakeCore()
    let drafts = try makeTemporaryDirectory().appending(path: "drafts.json")
    let resends = PromptResends()
    let clock = TestClock()
    let model = await retryingModel(core, clock, resends: resends, draftsFile: drafts) { clock.sleep($0) }
    model.draft = "fix the build"
    core.set(hold: true, error: unreachable)
    let send = Task { await model.sendPrompt() }
    await core.waitHeld(1)
    await leave(model, core)
    core.release()
    await send.value
    #expect(core.snapshot.prompts.count == 5)
    #expect(AgentDrafts.load(from: drafts).drafts[route("term_1")]?.text == "fix the build")

    let reopened = AgentModel(core: core, route: route("term_1"), prefsFile: nil, draftsFile: drafts, resends: resends)
    #expect(reopened.draft == "fix the build")
    #expect(reopened.promptError == notSent)
    let again = AgentModel(core: core, route: route("term_1"), prefsFile: nil, draftsFile: drafts, resends: resends)
    #expect(again.promptError == nil)
}

@MainActor
@Test func removingTheMachineDropsItsResends() async {
    let core = FakeCore()
    let resends = PromptResends()
    let (model, send) = await waitingModel(core, resends: resends)
    resends.stop(machineId: "m1")
    await send.value
    #expect(resends.entries.isEmpty)
    #expect(core.snapshot.prompts == ["fix the build"])
    #expect(!model.retrying)
}

@MainActor
@Test func comingBackToTheTabRelinksTheScreen() async {
    let core = FakeCore()
    let resends = PromptResends()
    let (model, send) = await waitingModel(core, resends: resends)
    await leave(model, core)
    #expect(resends.entries[route("term_1")]?.viewer == nil)
    let run = Task { await model.run() }
    await poll { resends.entries[route("term_1")]?.viewer === model }
    core.set(error: nil)
    resends.wake(route("term_1"))
    await send.value
    run.cancel()
    await run.value
    #expect(model.draft.isEmpty)
    #expect(model.promptError == nil)
}

@MainActor
@Test func aSendClearsANoteLeftForALaterScreen() async {
    let core = FakeCore()
    let resends = PromptResends()
    let (model, send) = await waitingModel(core, resends: resends)
    await leave(model, core)
    resends.stop(route("term_1"))
    await send.value
    core.set(error: nil)
    await model.sendPrompt()
    #expect(core.snapshot.prompts == ["fix the build", "fix the build"])
    let reopened = AgentModel(core: core, route: route("term_1"), prefsFile: nil, resends: resends)
    #expect(reopened.promptError == nil)
}

@MainActor
@Test func aScreenOpenedDuringTheFirstAttemptShowsIt() async {
    let core = FakeCore()
    let resends = PromptResends()
    let first = await retryingModel(core, TestClock(), resends: resends) { _ in }
    first.draft = "fix the build"
    core.set(hold: true)
    let send = Task { await first.sendPrompt() }
    await core.waitHeld(1)
    await leave(first, core)
    let second = await retryingModel(core, TestClock(), resends: resends) { _ in }
    #expect(second.draft == "fix the build")
    #expect(second.sendingPrompt)
    #expect(!second.retrying)
    core.release()
    await send.value
    await poll { !second.sendingPrompt }
    #expect(second.draft.isEmpty)
    #expect(core.snapshot.prompts == ["fix the build"])
}

@MainActor
@Test func aSecondScreenSendingDuringTheFirstAttemptWaitsForIt() async {
    let core = FakeCore()
    let resends = PromptResends()
    let first = await retryingModel(core, TestClock(), resends: resends) { _ in }
    first.draft = "fix the build"
    core.set(hold: true, error: unreachable)
    let firstSend = Task { await first.sendPrompt() }
    await core.waitHeld(1)
    let second = AgentModel(core: core, route: route("term_1"), prefsFile: nil, resends: resends)
    second.draft = "something else"
    let secondSend = Task { await second.sendPrompt() }
    await poll { resends.entries[route("term_1")]?.stopped == true }
    #expect(core.snapshot.prompts == ["fix the build"], "nothing else is sent while the first is in flight")
    core.set(error: nil)
    core.release()
    await firstSend.value
    await secondSend.value
    #expect(core.snapshot.prompts == ["fix the build", "something else"])
    #expect(core.snapshot.resends.isEmpty)
}

@MainActor
@Test func aFieldFilledFromTheMacStillShowsThePendingPrompt() async {
    let core = FakeCore()
    let resends = PromptResends()
    core.state.withLock { $0.macDraft = "typed on the Mac" }
    let other = AgentModel(core: core, route: route("term_1"), prefsFile: nil, resends: resends)
    core.state.withLock { $0.kind = "claude" }
    other.poll()
    await other.loadMacDraft()
    #expect(other.draft == "typed on the Mac")
    let first = AgentModel(core: core, route: route("term_1"), prefsFile: nil, resends: resends)
    first.poll()
    first.draft = "fix the build"
    core.set(hold: true)
    let send = Task { await first.sendPrompt() }
    await core.waitHeld(1)
    await leave(first, core)
    other.poll()
    #expect(other.draft == "fix the build")
    #expect(other.sendingPrompt)
    core.release()
    await send.value
}
