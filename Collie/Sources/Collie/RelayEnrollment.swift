import CryptoKit
import DeviceCheck
import Foundation
import Security

struct RelayRouteGrant: Codable, Equatable, Sendable {
    let token: String
    let routeId: String
    let generation: UInt32
    let expiresAt: UInt64

    var needsRenewal: Bool { expiresAt <= UInt64(Date().timeIntervalSince1970) + 7 * 86400 }
}

struct RelayRoute: Codable, Equatable, Sendable {
    let pairingId: String
    let activityId: String?
    let destination: String
    let grant: RelayRouteGrant
}

struct RelayPendingRoute: Codable, Sendable {
    let pairingId: String
    let activityId: String?
    let destination: String
    let routeId: String
    let expectedGeneration: UInt32
    let expiresAt: UInt64
}

struct RelayPendingRouteRevocation: Codable, Sendable {
    let pairingId: String
    let routeId: String
    let generation: UInt32
    let expiresAt: UInt64
}

struct RelayEnrollmentState: Codable, Sendable {
    var installationId = UUID().uuidString.lowercased()
    var keyId: String?
    var enrolled = false
    var routes: [RelayRoute] = []
    var pendingRoutes: [RelayPendingRoute] = []
    var attestationAttempted = false
    var requiresPairing = false
    var previousGrantExpiry: UInt64?
    var pendingRevocationExpiry: [String: UInt64] = [:]
    var pendingRouteRevocations: [RelayPendingRouteRevocation] = []
    var installationRevocationExpiry: UInt64?
    var pendingPairingRevocations: [String] = []
    var pendingInstallationRevocation = false
}

enum RelayEnrollmentError: Error {
    case unavailable, unauthorized, rejected, storage, invalidResponse, removedPairing, pairAgain
}

protocol RelayAttesting: Sendable {
    func supported() async -> Bool
    func generateKey() async throws -> String
    func attest(keyId: String, hash: Data) async throws -> Data
    func assert(keyId: String, hash: Data) async throws -> Data
}

struct DeviceRelayAttestation: RelayAttesting {
    func supported() async -> Bool { DCAppAttestService.shared.isSupported }
    func generateKey() async throws -> String { try await DCAppAttestService.shared.generateKey() }
    func attest(keyId: String, hash: Data) async throws -> Data {
        try await DCAppAttestService.shared.attestKey(keyId, clientDataHash: hash)
    }
    func assert(keyId: String, hash: Data) async throws -> Data {
        try await DCAppAttestService.shared.generateAssertion(keyId, clientDataHash: hash)
    }
}

struct RelayControlResponse: Decodable, Sendable {
    let v: Int
    let result: String
    let challenge: String?
    let grant: String?
    let routeId: String?
    let generation: UInt32?
    let expiresAt: UInt64?

    enum CodingKeys: String, CodingKey {
        case v, result, challenge, grant, generation
        case routeId = "route_id"
        case expiresAt = "expires_at"
    }
}

protocol RelayControlTransport: Sendable {
    func post(path: String, body: Data) async throws -> RelayControlResponse
}

private final class RelayRedirects: NSObject, URLSessionTaskDelegate, Sendable {
    func urlSession(
        _ session: URLSession, task: URLSessionTask, willPerformHTTPRedirection response: HTTPURLResponse,
        newRequest request: URLRequest, completionHandler: @escaping @Sendable (URLRequest?) -> Void
    ) {
        completionHandler(nil)
    }
}

struct PublisherRelayTransport: RelayControlTransport {
    private static let endpoint = URL(string: "https://collie-push.rbstp.dev")!

    func post(path: String, body: Data) async throws -> RelayControlResponse {
        guard ["challenge", "control"].contains(path), body.count <= 16384 else { throw RelayEnrollmentError.invalidResponse }
        var request = URLRequest(url: Self.endpoint.appending(path: "v1/\(path)"))
        request.httpMethod = "POST"
        request.httpBody = body
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        request.timeoutInterval = 15
        let configuration = URLSessionConfiguration.ephemeral
        configuration.httpCookieStorage = nil
        configuration.urlCache = nil
        let session = URLSession(configuration: configuration, delegate: RelayRedirects(), delegateQueue: nil)
        defer { session.invalidateAndCancel() }
        let (stream, response) = try await session.bytes(for: request)
        guard let response = response as? HTTPURLResponse else { throw RelayEnrollmentError.invalidResponse }
        if response.statusCode == 401 { throw RelayEnrollmentError.unauthorized }
        if response.statusCode == 503 { throw RelayEnrollmentError.unavailable }
        guard response.statusCode == 200, response.expectedContentLength <= 8192 else { throw RelayEnrollmentError.rejected }
        var data = Data()
        for try await byte in stream {
            guard data.count < 8192 else { throw RelayEnrollmentError.invalidResponse }
            data.append(byte)
        }
        let value = try JSONDecoder().decode(RelayControlResponse.self, from: data)
        guard value.v == 1 else { throw RelayEnrollmentError.invalidResponse }
        return value
    }
}

struct RelayCredentialStore: Sendable {
    let marker: URL

    private var query: [String: Any] {
        [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: "dev.rbstp.collie.relay",
            kSecAttrAccount as String: "installation",
            kSecUseDataProtectionKeychain as String: true,
            kSecAttrSynchronizable as String: false,
        ]
    }

    func load() throws -> RelayEnrollmentState {
        guard FileManager.default.fileExists(atPath: marker.path) else { return RelayEnrollmentState() }
        let markerId = try String(contentsOf: marker, encoding: .utf8)
        var read = query
        read[kSecReturnData as String] = true
        read[kSecMatchLimit as String] = kSecMatchLimitOne
        var result: CFTypeRef?
        let status = SecItemCopyMatching(read as CFDictionary, &result)
        guard status == errSecSuccess, let data = result as? Data else { throw RelayEnrollmentError.storage }
        let state = try JSONDecoder().decode(RelayEnrollmentState.self, from: data)
        guard state.installationId == markerId else { throw RelayEnrollmentError.storage }
        return state
    }

    func save(_ state: RelayEnrollmentState) throws {
        let data = try JSONEncoder().encode(state)
        var item = query
        item[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
        item[kSecValueData as String] = data
        let updated = SecItemUpdate(query as CFDictionary, [kSecValueData as String: data] as CFDictionary)
        guard updated == errSecSuccess || (updated == errSecItemNotFound && SecItemAdd(item as CFDictionary, nil) == errSecSuccess)
        else { throw RelayEnrollmentError.storage }
        try Data(state.installationId.utf8).write(to: marker, options: [.atomic, .completeFileProtectionUntilFirstUserAuthentication])
        var target = marker
        var values = URLResourceValues()
        values.isExcludedFromBackup = true
        try target.setResourceValues(values)
    }
}

actor RelayControlGate {
    private var busy = false
    private var waiting: [CheckedContinuation<Void, Never>] = []

    func enter() async {
        if busy { await withCheckedContinuation { waiting.append($0) } } else { busy = true }
    }

    func leave() {
        if waiting.isEmpty { busy = false } else { waiting.removeFirst().resume() }
    }
}

actor RelayEnrollment {
    private var state: RelayEnrollmentState
    private let attestation: any RelayAttesting
    private let transport: any RelayControlTransport
    private let persist: @Sendable (RelayEnrollmentState) throws -> Void
    private let gate = RelayControlGate()
    private let environment: String

    init(
        state: RelayEnrollmentState, environment: String, attestation: any RelayAttesting = DeviceRelayAttestation(),
        transport: any RelayControlTransport = PublisherRelayTransport(), persist: @escaping @Sendable (RelayEnrollmentState) throws -> Void
    ) {
        self.state = state
        self.environment = environment
        self.attestation = attestation
        self.transport = transport
        self.persist = persist
    }

    func snapshot() -> RelayEnrollmentState { state }

    func register(pairingId: String, activityId: String? = nil, destination: String) async throws -> RelayRouteGrant {
        await gate.enter()
        do {
            try Task.checkCancellation()
            guard !state.requiresPairing else { throw RelayEnrollmentError.pairAgain }
            try await retryRevocations()
            guard !state.pendingPairingRevocations.contains(pairingId), !state.pendingInstallationRevocation else {
                throw RelayEnrollmentError.removedPairing
            }
            try await enroll()
            if let pending = state.pendingRoutes.first(where: { $0.pairingId == pairingId && $0.activityId == activityId }),
                pending.destination != destination
            {
                _ = try await registerRoute(pairingId: pairingId, activityId: activityId, destination: pending.destination)
            }
            let grant = try await registerRoute(pairingId: pairingId, activityId: activityId, destination: destination)
            await gate.leave()
            return grant
        } catch {
            await gate.leave()
            throw error
        }
    }

    private var lastGrantExpiry: UInt64 {
        (state.routes.map(\.grant.expiresAt) + state.pendingRoutes.map(\.expiresAt)
            + Array(state.pendingRevocationExpiry.values) + state.pendingRouteRevocations.map(\.expiresAt)
            + [state.previousGrantExpiry ?? 0]).max() ?? 0
    }

    private func registerRoute(pairingId: String, activityId: String?, destination: String) async throws -> RelayRouteGrant {
        let expired = lastGrantExpiry <= UInt64(Date().timeIntervalSince1970)
        do { return try await issueRoute(pairingId: pairingId, activityId: activityId, destination: destination) }
        catch RelayEnrollmentError.unauthorized where expired {
            state = RelayEnrollmentState()
            try persist(state)
            try await enroll()
            return try await issueRoute(pairingId: pairingId, activityId: activityId, destination: destination)
        }
    }

    private func issueRoute(pairingId: String, activityId: String?, destination: String) async throws -> RelayRouteGrant {
        let old = state.routes.first { $0.pairingId == pairingId && $0.activityId == activityId }
        if let old, old.destination == destination, !old.grant.needsRenewal {
            return old.grant
        }
        let pending = state.pendingRoutes.first { $0.pairingId == pairingId && $0.activityId == activityId }
        let routeId = pending?.routeId ?? old?.grant.routeId ?? UUID().uuidString.lowercased()
        let expectedGeneration = pending?.expectedGeneration ?? old?.grant.generation ?? 0
        if pending == nil {
            state.pendingRoutes.append(RelayPendingRoute(pairingId: pairingId, activityId: activityId, destination: destination,
                                   routeId: routeId, expectedGeneration: expectedGeneration, expiresAt: UInt64(Date().timeIntervalSince1970) + 30 * 86400))
            try persist(state)
        }
        var operation = try common("route")
        operation["pairing_id"] = .string(wirePairingId(pairingId))
        operation["route_id"] = .string(routeId)
        operation["expected_generation"] = .number(expectedGeneration)
        operation["kind"] = .string(activityId == nil ? "app" : "liveactivity")
        operation["destination"] = .string(destination)
        let result = try await control(operation)
        guard result.result == "registered", let token = result.grant, let returnedId = result.routeId,
            returnedId == routeId, let generation = result.generation, generation == expectedGeneration + 1,
            let expiresAt = result.expiresAt, token.count <= 2048
        else { throw RelayEnrollmentError.invalidResponse }
        let grant = RelayRouteGrant(token: token, routeId: routeId, generation: generation, expiresAt: expiresAt)
        state.routes.removeAll { $0.pairingId == pairingId && $0.activityId == activityId }
        state.pendingRoutes.removeAll { $0.pairingId == pairingId && $0.activityId == activityId }
        state.routes.append(RelayRoute(pairingId: pairingId, activityId: activityId, destination: destination, grant: grant))
        try persist(state)
        return grant
    }

    func prepareRevocation(pairingId: String) async throws {
        await gate.enter()
        do {
            var next = state
            let expiry = (state.routes.filter { $0.pairingId == pairingId }.map(\.grant.expiresAt)
                + state.pendingRoutes.filter { $0.pairingId == pairingId }.map(\.expiresAt)).max() ?? UInt64(Date().timeIntervalSince1970)
            if !next.pendingPairingRevocations.contains(pairingId) { next.pendingPairingRevocations.append(pairingId) }
            next.pendingRevocationExpiry[pairingId] = max(next.pendingRevocationExpiry[pairingId] ?? 0, expiry)
            next.routes.removeAll { $0.pairingId == pairingId }
            next.pendingRoutes.removeAll { $0.pairingId == pairingId }
            try persist(next)
            state = next
            await gate.leave()
        } catch { await gate.leave(); throw error }
    }

    func revoke(pairingId: String) async throws {
        try await prepareRevocation(pairingId: pairingId)
        try await retryPendingRevocations()
    }

    func confirmNewPairing() async throws {
        await gate.enter()
        do {
            if state.requiresPairing {
                let expiry = lastGrantExpiry
                state = RelayEnrollmentState()
                state.previousGrantExpiry = expiry
                try persist(state)
            }
            await gate.leave()
        } catch { await gate.leave(); throw error }
    }

    func revokeActivity(pairingId: String, activityId: String) async throws {
        await gate.enter()
        do {
            for route in state.routes where route.pairingId == pairingId && route.activityId == activityId {
                state.pendingRouteRevocations.append(RelayPendingRouteRevocation(pairingId: pairingId, routeId: route.grant.routeId,
                    generation: route.grant.generation, expiresAt: route.grant.expiresAt))
            }
            for route in state.pendingRoutes where route.pairingId == pairingId && route.activityId == activityId {
                state.pendingRouteRevocations.append(RelayPendingRouteRevocation(pairingId: pairingId, routeId: route.routeId,
                    generation: route.expectedGeneration + 1, expiresAt: route.expiresAt))
            }
            state.routes.removeAll { $0.pairingId == pairingId && $0.activityId == activityId }
            state.pendingRoutes.removeAll { $0.pairingId == pairingId && $0.activityId == activityId }
            try persist(state)
            try await retryRevocations()
            await gate.leave()
        } catch { await gate.leave(); throw error }
    }

    func revokeInstallation() async throws {
        await gate.enter()
        do {
            state.installationRevocationExpiry = lastGrantExpiry
            state.pendingInstallationRevocation = true
            state.routes.removeAll()
            state.pendingRoutes.removeAll()
            try persist(state)
            try await retryRevocations()
            await gate.leave()
        } catch {
            await gate.leave()
            throw error
        }
    }

    func retryPendingRevocations() async throws {
        await gate.enter()
        do { try await retryRevocations(); await gate.leave() }
        catch { await gate.leave(); throw error }
    }

    private enum Value: Encodable {
        case string(String), number(UInt32)
        func encode(to encoder: Encoder) throws {
            var container = encoder.singleValueContainer()
            switch self {
            case .string(let value): try container.encode(value)
            case .number(let value): try container.encode(value)
            }
        }
    }

    private func wirePairingId(_ pairingId: String) -> String {
        SHA256.hash(data: Data("collie-relay-pairing-v1\n\(state.installationId)\n\(pairingId)".utf8))
            .map { String(format: "%02x", $0) }.joined()
    }

    private func common(_ name: String) throws -> [String: Value] {
        guard let keyId = state.keyId, let key = Data(base64Encoded: keyId), key.count == 32 else { throw RelayEnrollmentError.unavailable }
        return ["v": .number(1), "op": .string(name), "installation_id": .string(state.installationId),
                "key_id": .string(key.base64URLEncoded), "environment": .string(environment)]
    }

    private func enroll() async throws {
        guard !state.enrolled else { return }
        guard await attestation.supported() else { throw RelayEnrollmentError.unavailable }
        if state.keyId == nil { state.keyId = try await attestation.generateKey(); try persist(state) }
        if state.attestationAttempted {
            do {
                let recovered = try await control(common("enroll"))
                guard recovered.result == "enrolled" else { throw RelayEnrollmentError.rejected }
                state.enrolled = true
                try persist(state)
                return
            } catch RelayEnrollmentError.unauthorized {
                state = RelayEnrollmentState()
                state.keyId = try await attestation.generateKey()
                try persist(state)
            }
        }
        let result = try await control(common("enroll"), enrolling: true)
        guard result.result == "enrolled" else { throw RelayEnrollmentError.rejected }
        state.enrolled = true
        try persist(state)
    }

    private func control(_ operation: [String: Value], enrolling: Bool = false) async throws -> RelayControlResponse {
        let encoder = JSONEncoder()
        let operationData = try encoder.encode(operation)
        let challenge = try await transport.post(path: "challenge", body: operationData)
        guard challenge.result == "challenge", let value = challenge.challenge, value.count <= 2048, let keyId = state.keyId else {
            throw RelayEnrollmentError.invalidResponse
        }
        let hash = Data(SHA256.hash(data: Data(value.utf8)))
        let proof: Data
        do { proof = try await enrolling ? attestation.attest(keyId: keyId, hash: hash) : attestation.assert(keyId: keyId, hash: hash) }
        catch let error as DCError where error.code == .invalidKey {
            state.requiresPairing = true
            state.previousGrantExpiry = lastGrantExpiry
            try persist(state)
            throw RelayEnrollmentError.pairAgain
        }
        if enrolling { state.attestationAttempted = true; try persist(state) }
        guard proof.count <= 12000 else { throw RelayEnrollmentError.invalidResponse }
        struct Body: Encodable {
            let operation: [String: Value]
            let challenge: String
            let attestation: String?
            let assertion: String?
        }
        let body = Body(operation: operation, challenge: value, attestation: enrolling ? proof.base64URLEncoded : nil,
                        assertion: enrolling ? nil : proof.base64URLEncoded)
        return try await transport.post(path: "control", body: encoder.encode(body))
    }

    private func retryRevocations() async throws {
        let now = UInt64(Date().timeIntervalSince1970)
        let previousCount = state.pendingPairingRevocations.count + state.pendingRouteRevocations.count
        let previousExpiry = state.previousGrantExpiry
        state.pendingPairingRevocations.removeAll { (state.pendingRevocationExpiry[$0] ?? UInt64.max) <= now }
        state.pendingRevocationExpiry = state.pendingRevocationExpiry.filter { $0.value > now }
        state.pendingRouteRevocations.removeAll { $0.expiresAt <= now }
        if let expiry = state.previousGrantExpiry, expiry <= now { state.previousGrantExpiry = nil }
        if state.pendingInstallationRevocation, (state.installationRevocationExpiry ?? UInt64.max) <= now {
            state = RelayEnrollmentState()
            try persist(state)
            return
        }
        if previousCount != state.pendingPairingRevocations.count + state.pendingRouteRevocations.count || previousExpiry != state.previousGrantExpiry {
            try persist(state)
        }
        guard state.enrolled else {
            guard !state.pendingPairingRevocations.isEmpty || !state.pendingRouteRevocations.isEmpty || state.pendingInstallationRevocation else { return }
            state.pendingPairingRevocations.removeAll()
            state.pendingRevocationExpiry.removeAll()
            state.pendingRouteRevocations.removeAll()
            state.pendingInstallationRevocation = false
            try persist(state)
            return
        }
        if state.pendingInstallationRevocation {
            let result = try await control(common("revoke_installation"))
            guard result.result == "revoked" else { throw RelayEnrollmentError.rejected }
            state = RelayEnrollmentState()
            try persist(state)
            return
        }
        while let pairingId = state.pendingPairingRevocations.first {
            var operation = try common("revoke_pairing")
            operation["pairing_id"] = .string(wirePairingId(pairingId))
            let result = try await control(operation)
            guard result.result == "revoked" else { throw RelayEnrollmentError.rejected }
            state.pendingPairingRevocations.removeFirst()
            state.pendingRevocationExpiry.removeValue(forKey: pairingId)
            try persist(state)
        }
        while let route = state.pendingRouteRevocations.first {
            var operation = try common("revoke_route")
            operation["pairing_id"] = .string(wirePairingId(route.pairingId))
            operation["route_id"] = .string(route.routeId)
            operation["expected_generation"] = .number(route.generation)
            let result = try await control(operation)
            guard result.result == "revoked" else { throw RelayEnrollmentError.rejected }
            state.pendingRouteRevocations.removeFirst()
            try persist(state)
        }
    }
}

private extension Data {
    var base64URLEncoded: String {
        base64EncodedString().replacingOccurrences(of: "+", with: "-").replacingOccurrences(of: "/", with: "_")
            .replacingOccurrences(of: "=", with: "")
    }
}
