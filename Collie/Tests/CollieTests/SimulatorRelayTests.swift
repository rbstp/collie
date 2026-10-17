#if DEBUG && targetEnvironment(simulator)
import CryptoKit
import Foundation
import XCTest
@testable import Collie

@MainActor
final class SimulatorRelayTests: XCTestCase {
    func testSoftwareProofBindsChallengeAndPersistsCounterAcrossActors() async throws {
        let directory = FileManager.default.temporaryDirectory.appending(path: UUID().uuidString)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        let counter = directory.appending(path: "counter")
        let key = P256.Signing.PrivateKey()
        let hash = Data(SHA256.hash(data: Data("local-challenge".utf8)))
        for expected in UInt32(1)...2 {
            let signer = try SimulatorRelayAttestation(privateKey: key.rawRepresentation, teamId: "TESTTEAM01", counterFile: counter)
            let proof = try await signer.assert(keyId: "unused", hash: hash)
            var remaining = proof
            XCTAssertEqual(remaining.removeFirst(), 0xa2)
            func item(_ kind: UInt8) throws -> Data {
                let header = try XCTUnwrap(remaining.first)
                remaining.removeFirst()
                XCTAssertEqual(header & 0xe0, kind)
                let count = header & 31 == 24 ? Int(remaining.removeFirst()) : Int(header & 31)
                guard remaining.count >= count else { throw RelayEnrollmentError.invalidResponse }
                let value = remaining.prefix(count)
                remaining.removeFirst(count)
                return Data(value)
            }
            XCTAssertEqual(try item(0x60), Data("signature".utf8))
            let signature = try P256.Signing.ECDSASignature(derRepresentation: item(0x40))
            XCTAssertEqual(try item(0x60), Data("authenticatorData".utf8))
            let auth = try item(0x40)
            XCTAssertTrue(remaining.isEmpty)
            XCTAssertEqual(auth.prefix(32), Data(SHA256.hash(data: Data("TESTTEAM01.dev.rbstp.collie".utf8))))
            XCTAssertEqual(auth.suffix(4).reduce(UInt32(0)) { $0 << 8 | UInt32($1) }, expected)
            XCTAssertTrue(key.publicKey.isValidSignature(signature, for: Data(SHA256.hash(data: auth + hash))))
            XCTAssertFalse(key.publicKey.isValidSignature(signature, for: auth + hash))
            XCTAssertFalse(key.publicKey.isValidSignature(signature, for: Data(SHA256.hash(data: auth + Data(repeating: 0, count: 32)))))
            XCTAssertEqual(try JSONDecoder().decode(UInt32.self, from: Data(contentsOf: counter)), expected)
        }
        try JSONEncoder().encode(UInt32.max).write(to: counter)
        let signer = try SimulatorRelayAttestation(privateKey: key.rawRepresentation, teamId: "TESTTEAM01", counterFile: counter)
        do { _ = try await signer.assert(keyId: "unused", hash: hash); XCTFail("Exhausted counters must fail") }
        catch RelayEnrollmentError.unavailable {}
    }
}
#endif
