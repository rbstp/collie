import Foundation
import XCTest
@testable import Collie

private actor RelayAttestationFixture: RelayAttesting {
    private(set) var assertions = 0
    func supported() -> Bool { true }
    func generateKey() -> String { Data(repeating: 1, count: 32).base64EncodedString() }
    func attest(keyId: String, hash: Data) -> Data { Data([1]) }
    func assert(keyId: String, hash: Data) -> Data { assertions += 1; return Data([2]) }
}

private actor RelayTransportFixture: RelayControlTransport {
    private(set) var calls: [String] = []
    private var firstChallenge: CheckedContinuation<Void, Never>?
    private let holdFirst: Bool
    private var failRevocation: Bool
    private var first = true

    init(holdFirst: Bool = false, failRevocation: Bool = false) {
        self.holdFirst = holdFirst
        self.failRevocation = failRevocation
    }

    func release() { firstChallenge?.resume(); firstChallenge = nil }

    func post(path: String, body: Data) async throws -> RelayControlResponse {
        let object = try XCTUnwrap(JSONSerialization.jsonObject(with: body) as? [String: Any])
        let operation = path == "challenge" ? object : try XCTUnwrap(object["operation"] as? [String: Any])
        let name = try XCTUnwrap(operation["op"] as? String)
        calls.append("\(path):\(name)")
        if path == "challenge" {
            if first, holdFirst {
                first = false
                await withCheckedContinuation { firstChallenge = $0 }
            }
            return RelayControlResponse(v: 1, result: "challenge", challenge: "c1.fixture.challenge.signature", grant: nil,
                                        routeId: nil, generation: nil, expiresAt: nil)
        }
        if name == "revoke_pairing", failRevocation { throw URLError(.notConnectedToInternet) }
        if name == "route" {
            return RelayControlResponse(v: 1, result: "registered", challenge: nil, grant: "v1.fixture.claims.signature",
                                        routeId: operation["route_id"] as? String,
                                        generation: UInt32((operation["expected_generation"] as? Int ?? 0) + 1),
                                        expiresAt: UInt64(Date().timeIntervalSince1970) + 30 * 86400)
        }
        return RelayControlResponse(v: 1, result: name == "enroll" ? "enrolled" : "revoked", challenge: nil, grant: nil,
                                    routeId: nil, generation: nil, expiresAt: nil)
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
}
