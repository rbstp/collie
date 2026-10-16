#if DEBUG && targetEnvironment(simulator)
import CryptoKit
import Foundation

private struct SimulatorRelayIdentity: Decodable {
    let installationId: String
    let keyId: String
    let privateKey: String
    let teamId: String
}

actor SimulatorRelayAttestation: RelayAttesting {
    private let key: P256.Signing.PrivateKey
    private let appId: String
    private let counterFile: URL

    init(privateKey: Data, teamId: String, counterFile: URL) throws {
        key = try P256.Signing.PrivateKey(rawRepresentation: privateKey)
        appId = "\(teamId).dev.rbstp.collie"
        self.counterFile = counterFile
    }

    func supported() -> Bool { true }
    func generateKey() throws -> String { throw RelayEnrollmentError.unavailable }
    func attest(keyId: String, hash: Data) throws -> Data { throw RelayEnrollmentError.unavailable }

    func assert(keyId: String, hash: Data) throws -> Data {
        let previous = try FileManager.default.fileExists(atPath: counterFile.path)
            ? JSONDecoder().decode(UInt32.self, from: Data(contentsOf: counterFile)) : 0
        guard previous < UInt32.max else { throw RelayEnrollmentError.unavailable }
        let counter = previous + 1
        try JSONEncoder().encode(counter).write(to: counterFile, options: [.atomic, .completeFileProtectionUntilFirstUserAuthentication])
        var auth = Data(SHA256.hash(data: Data(appId.utf8)))
        auth.append(0)
        auth.append(contentsOf: [UInt8(counter >> 24), UInt8(truncatingIfNeeded: counter >> 16),
                                UInt8(truncatingIfNeeded: counter >> 8), UInt8(truncatingIfNeeded: counter)])
        let signature = try key.signature(for: auth + hash).derRepresentation
        func item(_ data: Data, kind: UInt8) -> Data {
            let header: Data = data.count < 24 ? Data([kind | UInt8(data.count)]) : Data([kind | 24, UInt8(data.count)])
            return header + data
        }
        return Data([0xa2]) + item(Data("signature".utf8), kind: 0x60) + item(signature, kind: 0x40)
            + item(Data("authenticatorData".utf8), kind: 0x60) + item(auth, kind: 0x40)
    }
}

private struct SimulatorRelayTransport: RelayControlTransport {
    func post(path: String, body: Data) async throws -> RelayControlResponse {
        guard ["challenge", "control"].contains(path), body.count <= 16384 else { throw RelayEnrollmentError.invalidResponse }
        var request = URLRequest(url: URL(string: "http://127.0.0.1:8788/v1/\(path)")!)
        request.httpMethod = "POST"
        request.httpBody = body
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        request.timeoutInterval = 10
        let session = URLSession(configuration: .ephemeral, delegate: RelayRedirects(), delegateQueue: nil)
        defer { session.invalidateAndCancel() }
        let (stream, response) = try await session.bytes(for: request)
        guard let response = response as? HTTPURLResponse else { throw RelayEnrollmentError.invalidResponse }
        if response.statusCode == 401 { throw RelayEnrollmentError.unauthorized }
        guard response.statusCode == 200 else { throw RelayEnrollmentError.unavailable }
        var data = Data()
        for try await byte in stream {
            guard data.count < 8192 else { throw RelayEnrollmentError.invalidResponse }
            data.append(byte)
        }
        let result = try JSONDecoder().decode(RelayControlResponse.self, from: data)
        guard result.v == 1 else { throw RelayEnrollmentError.invalidResponse }
        return result
    }
}

enum SimulatorRelay {
    static func make() throws -> RelayEnrollment? {
        guard ["mock", "sandbox"].contains(ProcessInfo.processInfo.environment["COLLIE_SIMULATOR_RELAY"] ?? "") else { return nil }
        let directory = try StateDirectory.prepare()
        let identity = try JSONDecoder().decode(SimulatorRelayIdentity.self,
            from: Data(contentsOf: directory.appending(path: "simulator-relay.json")))
        func decode(_ value: String) -> Data? {
            let base = value.replacingOccurrences(of: "-", with: "+").replacingOccurrences(of: "_", with: "/")
            return Data(base64Encoded: base + String(repeating: "=", count: (4 - base.count % 4) % 4))
        }
        guard let key = decode(identity.privateKey), let keyId = decode(identity.keyId), keyId.count == 32,
            identity.teamId.count == 10 else { throw RelayEnrollmentError.storage }
        let stateFile = directory.appending(path: "simulator-relay-state.json")
        var state: RelayEnrollmentState
        if FileManager.default.fileExists(atPath: stateFile.path) {
            state = try JSONDecoder().decode(RelayEnrollmentState.self, from: Data(contentsOf: stateFile))
            guard state.installationId == identity.installationId else { throw RelayEnrollmentError.storage }
        } else {
            state = RelayEnrollmentState()
            state.installationId = identity.installationId
            state.keyId = keyId.base64EncodedString()
            state.enrolled = true
        }
        let attestation = try SimulatorRelayAttestation(privateKey: key, teamId: identity.teamId,
            counterFile: directory.appending(path: "simulator-relay-counter"))
        return RelayEnrollment(state: state, environment: "development", attestation: attestation,
            transport: SimulatorRelayTransport(), persist: { state in
                try JSONEncoder().encode(state).write(to: stateFile, options: [.atomic, .completeFileProtectionUntilFirstUserAuthentication])
            })
    }
}
#endif
