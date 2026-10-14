import CollieCore
import Foundation
import Synchronization
import Testing

@testable import Collie

private let unreachable = CoreError.Unreachable(message: "timed out waiting for the machine")

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
    _ core: FakeCore, _ clock: TestClock, sleep: @escaping @Sendable (Duration) async throws -> Void
) async -> AgentModel {
    core.state.withLock {
        $0.kind = "claude"
        $0.macDraft = ""
    }
    let model = AgentModel(
        core: core, route: route("term_1"), prefsFile: nil, now: { clock.now }, sleep: sleep
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
    #expect(clock.sleeps == Array(repeating: .seconds(60), count: 4))
    #expect(model.promptError == unreachable.description)
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
private func waitingModel(_ core: FakeCore) async -> (AgentModel, Task<Void, Never>) {
    core.set(error: unreachable)
    let model = await retryingModel(core, TestClock()) { try await Task.sleep(for: $0) }
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
    #expect(model.promptError == unreachable.description)
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
