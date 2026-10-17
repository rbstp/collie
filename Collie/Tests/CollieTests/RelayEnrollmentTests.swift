import Foundation
import DeviceCheck
import XCTest
@testable import Collie

private actor RelayAttestationFixture: RelayAttesting {
    private(set) var assertions = 0
    private var invalidKey = false
    private var holdAssertion: Bool
    private var pausedAssertion: CheckedContinuation<Void, Never>?
    init(holdAssertion: Bool = false) { self.holdAssertion = holdAssertion }
    func releaseAssertion() { pausedAssertion?.resume(); pausedAssertion = nil }
    func invalidate() { invalidKey = true }
    func supported() -> Bool { true }
    func generateKey() -> String { invalidKey = false; return Data(repeating: 2, count: 32).base64EncodedString() }
    func attest(keyId: String, hash: Data) -> Data { Data([1]) }
    func assert(keyId: String, hash: Data) async throws -> Data {
        if invalidKey { throw DCError(.invalidKey) }
        assertions += 1
        if holdAssertion {
            holdAssertion = false
            await withCheckedContinuation { pausedAssertion = $0 }
        }
        return Data([2])
    }
}

private actor RelayTransportFixture: RelayControlTransport {
    private(set) var calls: [String] = []
    private var firstChallenge: CheckedContinuation<Void, Never>?
    private let holdFirst: Bool
    private var failRevocation: Bool
    private var first = true
    private var challengeFailure: RelayEnrollmentError?
    private let challengeExpiresAt: UInt64
    private let failRouteResponse: Bool
    private var loseNextRouteResponse = false
    private var generations: [String: UInt32] = [:]
    private var revoked: Set<String> = []

    init(holdFirst: Bool = false, failRevocation: Bool = false, challengeFailure: RelayEnrollmentError? = nil,
         challengeExpiresAt: UInt64 = UInt64(Date().timeIntervalSince1970) + 300, failRouteResponse: Bool = false) {
        self.challengeExpiresAt = challengeExpiresAt
        self.failRouteResponse = failRouteResponse
        self.challengeFailure = challengeFailure
        self.holdFirst = holdFirst
        self.failRevocation = failRevocation
    }

    func release() { firstChallenge?.resume(); firstChallenge = nil }
    func loseRouteResponse() { loseNextRouteResponse = true }
    func failChallenge() { challengeFailure = .unavailable }
    func invalidate(_ grant: RelayRouteGrant) {
        if generations[grant.routeId] == grant.generation { revoked.insert(grant.routeId) }
    }
    func usable(_ grant: RelayRouteGrant) -> Bool {
        generations[grant.routeId] == grant.generation && !revoked.contains(grant.routeId)
    }

    func post(path: String, body: Data) async throws -> RelayControlResponse {
        let object = try XCTUnwrap(JSONSerialization.jsonObject(with: body) as? [String: Any])
        let operation = path == "challenge" ? object : try XCTUnwrap(object["operation"] as? [String: Any])
        let name = try XCTUnwrap(operation["op"] as? String)
        calls.append("\(path):\(name)")
        if path == "challenge" {
            if let failure = challengeFailure { challengeFailure = nil; throw failure }
            if first, holdFirst {
                first = false
                await withCheckedContinuation { firstChallenge = $0 }
            }
            return RelayControlResponse(v: 1, result: "challenge", challenge: "c1.fixture.challenge.signature", grant: nil,
                                        routeId: nil, generation: nil, expiresAt: challengeExpiresAt)
        }
        if name.hasPrefix("revoke_"), failRevocation { throw URLError(.notConnectedToInternet) }
        if name == "route" {
            let id = try XCTUnwrap(operation["route_id"] as? String)
            let expected = UInt32(try XCTUnwrap(operation["expected_generation"] as? Int))
            let current = generations[id] ?? expected
            guard current == expected || (current == expected + 1 && !revoked.contains(id)) else {
                throw RelayEnrollmentError.rejected
            }
            generations[id] = expected + 1
            revoked.remove(id)
            if failRouteResponse || loseNextRouteResponse {
                loseNextRouteResponse = false
                throw URLError(.networkConnectionLost)
            }
            return RelayControlResponse(v: 1, result: "registered", challenge: nil, grant: "v1.fixture.claims.signature",
                                        routeId: id, generation: expected + 1,
                                        expiresAt: UInt64(Date().timeIntervalSince1970) + 30 * 86400)
        }
        return RelayControlResponse(v: 1, result: name == "enroll" ? "enrolled" : "revoked", challenge: nil, grant: nil,
                                    routeId: nil, generation: nil, expiresAt: nil)
    }
}

private final class RelayPersistenceFixture: @unchecked Sendable {
    private let lock = NSLock()
    private var state: RelayEnrollmentState?
    private var failCompletion = false

    func failNextCompletion() {
        lock.lock()
        defer { lock.unlock() }
        failCompletion = true
    }

    func save(_ value: RelayEnrollmentState) throws {
        lock.lock()
        defer { lock.unlock() }
        if failCompletion, !value.routes.isEmpty, value.pendingRoutes.isEmpty {
            failCompletion = false
            throw RelayEnrollmentError.storage
        }
        state = value
    }

    func snapshot() -> RelayEnrollmentState? {
        lock.lock()
        defer { lock.unlock() }
        return state
    }
}

@MainActor
final class RelayEnrollmentTests: XCTestCase {
    private func enrolled() -> RelayEnrollmentState {
        var state = RelayEnrollmentState()
        state.keyId = Data(repeating: 1, count: 32).base64EncodedString()
        state.enrolled = true
        return state
    }

    func testCompleteTransactionsRemainSerializedAcrossActorSuspension() async throws {
        let transport = RelayTransportFixture(holdFirst: true)
        let attestation = RelayAttestationFixture()
        let relay = RelayEnrollment(state: enrolled(), environment: "development", attestation: attestation,
                                    transport: transport, persist: { _ in })
        let first = Task { try await relay.register(pairingId: "first", destination: String(repeating: "a", count: 64)) }
        while await transport.calls.isEmpty { await Task.yield() }
        let second = Task { try await relay.register(pairingId: "second", destination: String(repeating: "b", count: 64)) }
        for _ in 0..<10 { await Task.yield() }
        let heldCalls = await transport.calls
        XCTAssertEqual(heldCalls, ["challenge:route"])
        await transport.release()
        _ = try await first.value
        _ = try await second.value
        let calls = await transport.calls
        XCTAssertEqual(calls, ["challenge:route", "control:route", "challenge:route", "control:route"])
        let count = await attestation.assertions
        XCTAssertEqual(count, 2)
    }

    func testFreshGrantsAreReusedAndChangedDestinationsRotateIndependently() async throws {
        let transport = RelayTransportFixture()
        let relay = RelayEnrollment(state: enrolled(), environment: "development", attestation: RelayAttestationFixture(),
                                    transport: transport, persist: { _ in })
        let first = try await relay.register(pairingId: "first", destination: String(repeating: "a", count: 64))
        let reused = try await relay.register(pairingId: "first", destination: String(repeating: "a", count: 64))
        XCTAssertEqual(first, reused)
        let second = try await relay.register(pairingId: "second", destination: String(repeating: "b", count: 64))
        let rotated = try await relay.register(pairingId: "first", destination: String(repeating: "c", count: 64))
        XCTAssertEqual(rotated.routeId, first.routeId)
        XCTAssertEqual(rotated.generation, first.generation + 1)
        let state = await relay.snapshot()
        XCTAssertEqual(state.routes.first { $0.pairingId == "second" }?.grant, second)
    }

    func testOfflineRevocationRetainsOnlyRetryAuthorityAndShowsPendingState() async throws {
        let transport = RelayTransportFixture(failRevocation: true)
        var initial = enrolled()
        initial.routes = [RelayRoute(pairingId: "removed", activityId: nil, destination: String(repeating: "a", count: 64),
                                     grant: RelayRouteGrant(token: "secret", routeId: "route", generation: 1, expiresAt: 2000000000))]
        let relay = RelayEnrollment(state: initial, environment: "development", attestation: RelayAttestationFixture(),
                                    transport: transport, persist: { _ in })
        do { try await relay.revoke(pairingId: "removed"); XCTFail("Offline revocation unexpectedly succeeded") } catch {}
        let state = await relay.snapshot()
        XCTAssertTrue(state.routes.isEmpty)
        XCTAssertEqual(state.pendingPairingRevocations, ["removed"])
        XCTAssertEqual(state.keyId, initial.keyId)
    }

    func testRenewalStartsSevenDaysBeforeExpiry() {
        let now = UInt64(Date().timeIntervalSince1970)
        XCTAssertTrue(RelayRouteGrant(token: "token", routeId: "route", generation: 1, expiresAt: now + 7 * 86400 - 1).needsRenewal)
        XCTAssertFalse(RelayRouteGrant(token: "token", routeId: "route", generation: 1, expiresAt: now + 8 * 86400).needsRenewal)
    }
    func testExpiredDeletedServerStateCanEnrollAgainButTransientFailuresPreserveIdentity() async throws {
        let initial = enrolled()
        let expired = RelayEnrollment(state: initial, environment: "development", attestation: RelayAttestationFixture(),
                                      transport: RelayTransportFixture(challengeFailure: .unauthorized), persist: { _ in })
        _ = try await expired.register(pairingId: "paired", destination: String(repeating: "a", count: 64))
        let renewed = await expired.snapshot()
        XCTAssertNotEqual(renewed.installationId, initial.installationId)
        XCTAssertTrue(renewed.enrolled)
        let unavailable = RelayEnrollment(state: initial, environment: "development", attestation: RelayAttestationFixture(),
                                          transport: RelayTransportFixture(challengeFailure: .unavailable), persist: { _ in })
        do { _ = try await unavailable.register(pairingId: "paired", destination: String(repeating: "a", count: 64)); XCTFail("Unexpected success") } catch {}
        let retained = await unavailable.snapshot()
        XCTAssertEqual(retained.installationId, initial.installationId)
        XCTAssertEqual(retained.keyId, initial.keyId)
    }

    func testInvalidKeyRequiresNewPairingAndPreservesPreviousGrantExpiry() async throws {
        let attestation = RelayAttestationFixture()
        await attestation.invalidate()
        var initial = enrolled()
        let expiry = UInt64(Date().timeIntervalSince1970) + 86400
        initial.routes = [RelayRoute(pairingId: "paired", activityId: nil, destination: String(repeating: "a", count: 64),
                                     grant: RelayRouteGrant(token: "token", routeId: "route", generation: 1, expiresAt: expiry))]
        let relay = RelayEnrollment(state: initial, environment: "development", attestation: attestation,
                                    transport: RelayTransportFixture(), persist: { _ in })
        do { _ = try await relay.register(pairingId: "paired", destination: String(repeating: "b", count: 64)); XCTFail("Unexpected success") } catch {}
        let invalidated = await relay.snapshot()
        XCTAssertTrue(invalidated.requiresPairing)
        try await relay.confirmNewPairing()
        let reset = await relay.snapshot()
        XCTAssertFalse(reset.requiresPairing)
        XCTAssertGreaterThanOrEqual(reset.previousGrantExpiry ?? 0, expiry)
        _ = try await relay.register(pairingId: "new-pair", destination: String(repeating: "b", count: 64))
        let recovered = await relay.snapshot()
        XCTAssertTrue(recovered.enrolled)
        XCTAssertNotEqual(recovered.keyId, initial.keyId)
    }

    func testAmbiguousRouteAuthorityUsesDurableServerChallengeBoundBeforeAssertionSuspends() async throws {
        let serverExpiry = UInt64(Date().timeIntervalSince1970) + 2 * 86400 + 300
        let bound = serverExpiry + 30 * 86400
        let transport = RelayTransportFixture(failRevocation: true, challengeExpiresAt: serverExpiry, failRouteResponse: true)
        let attestation = RelayAttestationFixture(holdAssertion: true)
        let persistence = RelayPersistenceFixture()
        let relay = RelayEnrollment(state: enrolled(), environment: "development", attestation: attestation,
                                    transport: transport, persist: { try persistence.save($0) })
        let registration = Task { try await relay.register(pairingId: "paired", destination: String(repeating: "a", count: 64)) }
        while await attestation.assertions == 0 { await Task.yield() }
        let paused = await relay.snapshot()
        XCTAssertEqual(paused.pendingRoutes.first?.expiresAt, bound)
        XCTAssertEqual(persistence.snapshot()?.pendingRoutes.first?.expiresAt, bound)
        await attestation.releaseAssertion()
        do { _ = try await registration.value; XCTFail("A lost response unexpectedly succeeded") } catch {}
        try await relay.prepareRevocation(pairingId: "paired")
        do { try await relay.retryPendingRevocations(); XCTFail("Offline revocation unexpectedly succeeded") } catch {}
        let retained = await relay.snapshot()
        XCTAssertEqual(retained.pendingRevocationExpiry["paired"], bound)
        XCTAssertEqual(retained.pendingPairingRevocations, ["paired"])
        XCTAssertEqual(persistence.snapshot()?.pendingRevocationExpiry["paired"], bound)
    }

    func testRepeatedOfflineInstallationRevocationRetainsItsOriginalAuthorityUntilExpiry() async throws {
        var initial = enrolled()
        let expiry = UInt64(Date().timeIntervalSince1970) + 86400
        initial.routes = [RelayRoute(pairingId: "paired", activityId: nil, destination: String(repeating: "a", count: 64),
                                     grant: RelayRouteGrant(token: "token", routeId: "route", generation: 1, expiresAt: expiry))]
        let transport = RelayTransportFixture(failRevocation: true)
        let relay = RelayEnrollment(state: initial, environment: "development", attestation: RelayAttestationFixture(),
                                    transport: transport, persist: { _ in })
        for _ in 0..<2 {
            do { try await relay.revokeInstallation(); XCTFail("Offline revocation unexpectedly succeeded") } catch {}
            let retained = await relay.snapshot()
            XCTAssertTrue(retained.pendingInstallationRevocation)
            XCTAssertEqual(retained.installationRevocationExpiry, expiry)
            XCTAssertEqual(retained.installationId, initial.installationId)
            XCTAssertEqual(retained.keyId, initial.keyId)
        }
        var expired = initial
        expired.routes = []
        expired.pendingInstallationRevocation = true
        expired.installationRevocationExpiry = UInt64(Date().timeIntervalSince1970) - 86400
        let expiredTransport = RelayTransportFixture(failRevocation: true)
        let inactive = RelayEnrollment(state: expired, environment: "development", attestation: RelayAttestationFixture(),
                                       transport: expiredTransport, persist: { _ in })
        try await inactive.retryPendingRevocations()
        let cleared = await inactive.snapshot()
        XCTAssertFalse(cleared.pendingInstallationRevocation)
        XCTAssertNotEqual(cleared.installationId, expired.installationId)
        let calls = await expiredTransport.calls
        XCTAssertTrue(calls.isEmpty)
    }

    func testSameTokenEventReplacesRevokedGrantAndStaleFailurePreservesReplacement() async throws {
        let transport = RelayTransportFixture()
        let relay = RelayEnrollment(state: enrolled(), environment: "development", attestation: RelayAttestationFixture(), transport: transport, persist: { _ in })
        let destination = String(repeating: "a", count: 64)
        let first = try await relay.register(pairingId: "paired", destination: destination)
        await transport.invalidate(first)
        let cached = try await relay.register(pairingId: "paired", destination: destination)
        XCTAssertEqual(cached, first)
        let replacement = try await relay.register(pairingId: "paired", destination: destination, tokenEvent: 1)
        XCTAssertEqual(replacement.routeId, first.routeId)
        XCTAssertEqual(replacement.generation, first.generation + 1)
        await transport.invalidate(first)
        let usable = await transport.usable(replacement)
        XCTAssertTrue(usable)
        for _ in 0..<10 {
            let refreshed = try await relay.register(pairingId: "paired", destination: destination, tokenEvent: 1)
            XCTAssertEqual(refreshed, replacement)
        }
        let calls = await transport.calls
        XCTAssertEqual(calls, ["challenge:route", "control:route", "challenge:route", "control:route"])
    }

    func testTokenEventDuringSuspendedRegistrationSurvivesAndRepeatedEventCoalesces() async throws {
        let transport = RelayTransportFixture(holdFirst: true)
        let relay = RelayEnrollment(state: enrolled(), environment: "development", attestation: RelayAttestationFixture(),
                                    transport: transport, persist: { _ in })
        let destination = String(repeating: "a", count: 64)
        let first = Task { try await relay.register(pairingId: "paired", destination: destination, tokenEvent: 1) }
        while await transport.calls.isEmpty { await Task.yield() }
        let newer = Task { try await relay.register(pairingId: "paired", destination: destination, tokenEvent: 2) }
        let repeated = Task { try await relay.register(pairingId: "paired", destination: destination, tokenEvent: 2) }
        await transport.release()
        let original = try await first.value
        let latest = try await newer.value
        let coalesced = try await repeated.value
        XCTAssertEqual(latest.generation, original.generation + 1)
        XCTAssertEqual(coalesced, latest)
        let stale = try await relay.register(pairingId: "paired", destination: destination, tokenEvent: 1)
        XCTAssertEqual(stale, latest)
        let calls = await transport.calls
        XCTAssertEqual(calls.count, 4)
    }

    func testPartialMachineFailureRetriesOnlyUnfulfilledEvent() async throws {
        let transport = RelayTransportFixture()
        let relay = RelayEnrollment(state: enrolled(), environment: "development", attestation: RelayAttestationFixture(),
                                    transport: transport, persist: { _ in })
        let destination = String(repeating: "a", count: 64)
        _ = try await relay.register(pairingId: "first", destination: destination)
        let oldSecond = try await relay.register(pairingId: "second", destination: destination)
        let first = try await relay.register(pairingId: "first", destination: destination, tokenEvent: 1)
        await transport.failChallenge()
        do {
            _ = try await relay.register(pairingId: "second", destination: destination, tokenEvent: 1)
            XCTFail("Unavailable control unexpectedly succeeded")
        } catch {}
        let pending = await relay.snapshot()
        XCTAssertEqual(pending.routes.first { $0.pairingId == "second" }?.grant, oldSecond)
        let cachedFirst = try await relay.register(pairingId: "first", destination: destination, tokenEvent: 1)
        XCTAssertEqual(cachedFirst, first)
        let second = try await relay.register(pairingId: "second", destination: destination, tokenEvent: 1)
        XCTAssertEqual(second.generation, oldSecond.generation + 1)
        let calls = await transport.calls
        XCTAssertEqual(calls.count, 9)
    }

    func testLostResponseRecoversSameEventWithoutRotatingAgain() async throws {
        let transport = RelayTransportFixture()
        let relay = RelayEnrollment(state: enrolled(), environment: "development", attestation: RelayAttestationFixture(),
                                    transport: transport, persist: { _ in })
        let destination = String(repeating: "a", count: 64)
        let first = try await relay.register(pairingId: "paired", destination: destination)
        await transport.loseRouteResponse()
        do {
            _ = try await relay.register(pairingId: "paired", destination: destination, tokenEvent: 1)
            XCTFail("Lost response unexpectedly succeeded")
        } catch {}
        let recovered = try await relay.register(pairingId: "paired", destination: destination, tokenEvent: 1)
        XCTAssertEqual(recovered.generation, first.generation + 1)
        let state = await relay.snapshot()
        XCTAssertTrue(state.pendingRoutes.isEmpty)
        let calls = await transport.calls
        XCTAssertEqual(calls.count, 6)
    }

    func testOlderAmbiguousRequestDoesNotFulfillNewerTokenEvent() async throws {
        let transport = RelayTransportFixture()
        let relay = RelayEnrollment(state: enrolled(), environment: "development", attestation: RelayAttestationFixture(),
                                    transport: transport, persist: { _ in })
        let destination = String(repeating: "a", count: 64)
        let first = try await relay.register(pairingId: "paired", destination: destination)
        await transport.loseRouteResponse()
        do {
            _ = try await relay.register(pairingId: "paired", destination: destination, tokenEvent: 1)
            XCTFail("Lost response unexpectedly succeeded")
        } catch {}
        let refreshed = try await relay.register(pairingId: "paired", destination: destination, tokenEvent: 2)
        XCTAssertEqual(refreshed.generation, first.generation + 2)
        let calls = await transport.calls
        XCTAssertEqual(calls.count, 8)
    }

    func testRestoredPendingOperationPrecedesFreshProcessTokenEvent() async throws {
        let transport = RelayTransportFixture()
        let persistence = RelayPersistenceFixture()
        let relay = RelayEnrollment(state: enrolled(), environment: "development", attestation: RelayAttestationFixture(),
                                    transport: transport, persist: { try persistence.save($0) })
        let destination = String(repeating: "a", count: 64)
        await transport.loseRouteResponse()
        do {
            _ = try await relay.register(pairingId: "paired", destination: destination, tokenEvent: 1)
            XCTFail("Lost response unexpectedly succeeded")
        } catch {}
        let pending = try XCTUnwrap(persistence.snapshot())
        let restarted = RelayEnrollment(state: pending, environment: "development", attestation: RelayAttestationFixture(),
                                        transport: transport, persist: { try persistence.save($0) })
        let renewed = try await restarted.register(pairingId: "paired", destination: destination, tokenEvent: 1)
        XCTAssertEqual(renewed.generation, 2)
        let calls = await transport.calls
        XCTAssertEqual(calls.count, 6)
    }

    func testActivityEventsDoNotRenewOtherActivitiesOrAppRoutes() async throws {
        let transport = RelayTransportFixture()
        let relay = RelayEnrollment(state: enrolled(), environment: "development", attestation: RelayAttestationFixture(),
                                    transport: transport, persist: { _ in })
        let destination = String(repeating: "a", count: 64)
        let app = try await relay.register(pairingId: "paired", destination: destination, tokenEvent: 1)
        let first = try await relay.register(pairingId: "paired", activityId: "one", destination: destination, tokenEvent: 1)
        let other = try await relay.register(pairingId: "paired", activityId: "two", destination: destination, tokenEvent: 1)
        await transport.invalidate(first)
        let renewed = try await relay.register(pairingId: "paired", activityId: "one", destination: destination, tokenEvent: 2)
        XCTAssertEqual(renewed.generation, first.generation + 1)
        let unchangedApp = try await relay.register(pairingId: "paired", destination: destination, tokenEvent: 1)
        let unchangedOther = try await relay.register(pairingId: "paired", activityId: "two", destination: destination, tokenEvent: 1)
        XCTAssertEqual(unchangedApp, app)
        XCTAssertEqual(unchangedOther, other)
        try await relay.revokeActivity(pairingId: "paired", activityId: "one")
        let state = await relay.snapshot()
        XCTAssertFalse(state.routes.contains { $0.activityId == "one" })
        XCTAssertEqual(state.routes.count, 2)
    }

    func testFailedGrantPersistenceRetainsOldGrantAndRecoversPendingGeneration() async throws {
        let transport = RelayTransportFixture()
        let persistence = RelayPersistenceFixture()
        let relay = RelayEnrollment(state: enrolled(), environment: "development", attestation: RelayAttestationFixture(),
                                    transport: transport, persist: { try persistence.save($0) })
        let destination = String(repeating: "a", count: 64)
        let first = try await relay.register(pairingId: "paired", destination: destination)
        persistence.failNextCompletion()
        do {
            _ = try await relay.register(pairingId: "paired", destination: destination, tokenEvent: 1)
            XCTFail("Failed persistence unexpectedly succeeded")
        } catch RelayEnrollmentError.storage {}
        let pending = await relay.snapshot()
        XCTAssertEqual(pending.routes.first?.grant, first)
        XCTAssertEqual(pending.pendingRoutes.first?.expectedGeneration, first.generation)
        XCTAssertEqual(persistence.snapshot()?.pendingRoutes.first?.expectedGeneration, first.generation)
        let recovered = try await relay.register(pairingId: "paired", destination: destination, tokenEvent: 1)
        XCTAssertEqual(recovered.generation, first.generation + 1)
        let calls = await transport.calls
        XCTAssertEqual(calls.count, 6)
    }

}
