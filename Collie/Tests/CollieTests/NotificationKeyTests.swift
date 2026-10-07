import CryptoKit
import Foundation
import Testing

@testable import Collie

private struct Vector: Decodable {
    let key: Data
    let approvalId: String
    let enc: String
    let body: String
}

/// docs/protocol/notification-vector.json, written by the Rust side: both implementations must agree on it.
private func vector() throws -> Vector {
    let url = URL(filePath: #filePath).deletingLastPathComponent().appending(path: "../../../docs/protocol/notification-vector.json")
    let json = try JSONSerialization.jsonObject(with: Data(contentsOf: url)) as? [String: Any]
    let object = try #require(json)
    let keyHex = try #require(object["key_hex"] as? String)
    let plaintext = try #require(object["plaintext"] as? String)
    let body = try #require(
        (try JSONSerialization.jsonObject(with: Data(plaintext.utf8)) as? [String: Any])?["body"] as? String
    )
    return Vector(
        key: try #require(Data(hex: keyHex)),
        approvalId: try #require(object["approval_id"] as? String),
        enc: try #require(object["enc"] as? String),
        body: body
    )
}

/// docs/protocol/notification-vector.json's key, which the Live Activity fixture's `enc` is sealed with too.
let vectorKey = SymmetricKey(data: Data((1...32).map { UInt8($0) }))

private func seal(_ plaintext: String, approvalId: String = "apr_1", key: SymmetricKey = vectorKey) throws -> String {
    try ChaChaPoly.seal(Data(plaintext.utf8), using: key, authenticating: Data(approvalId.utf8)).combined.base64EncodedString()
}

@Test func sharedVectorOpens() throws {
    let v = try vector()
    #expect(v.key == Data((1...32).map { UInt8($0) }))
    #expect(v.approvalId == "apr_test")
    #expect(v.body == "Bash: echo hi")
    #expect(PushContext.open(v.enc, approvalId: v.approvalId, key: SymmetricKey(data: v.key)) == v.body)
    #expect(v.enc == "oKGio6Slpqeoqaqr7QAv18tqJkZ41BDjm3Mz/9NV7Tim7Fe0ZJoerlF+PbZlhhy7Ws9jjgfEGOzE+w==")
}

@Test func anythingWrongFallsBack() throws {
    let v = try vector()
    let key = SymmetricKey(data: v.key)
    #expect(PushContext.open(v.enc, approvalId: "apr_other", key: key) == nil)
    #expect(PushContext.open(v.enc, approvalId: v.approvalId, key: SymmetricKey(size: .bits256)) == nil)
    var tampered = try #require(Data(base64Encoded: v.enc))
    tampered[14] ^= 1
    #expect(PushContext.open(tampered.base64EncodedString(), approvalId: v.approvalId, key: key) == nil)
    #expect(PushContext.open(tampered.prefix(27).base64EncodedString(), approvalId: v.approvalId, key: key) == nil)
    #expect(PushContext.open("not base64!", approvalId: v.approvalId, key: key) == nil)
    #expect(PushContext.open("", approvalId: v.approvalId, key: key) == nil)
    #expect(PushContext.open(try seal(#"{"v":2,"body":"x"}"#), approvalId: "apr_1", key: vectorKey) == nil)
    #expect(PushContext.open(try seal(#"{"body":"x"}"#), approvalId: "apr_1", key: vectorKey) == nil)
    #expect(PushContext.open(try seal("Bash: x"), approvalId: "apr_1", key: vectorKey) == nil)
    #expect(PushContext.open(try seal(#"{"v":1,"body":" \u0007 "}"#), approvalId: "apr_1", key: vectorKey) == nil)
}

@Test func contextIsCleanedAndCapped() throws {
    let dirty = #"{"v":1,"body":"Bash: rm\u001b[31m x‮\u0000y\n"}"#
    #expect(PushContext.open(try seal(dirty), approvalId: "apr_1", key: vectorKey) == "Bash: rm[31m xy")
    let multi = #"{"v":1,"body":"Bash: echo safe\nrm -rf ~/x\r"}"#
    #expect(PushContext.open(try seal(multi), approvalId: "apr_1", key: vectorKey) == "Bash: echo safe\nrm -rf ~/x")
    let long = String(repeating: "a", count: 700)
    #expect(PushContext.open(try seal(#"{"v":1,"body":"\#(long)"}"#), approvalId: "apr_1", key: vectorKey)?.count == 600)
}

extension Data {
    fileprivate init?(hex: String) {
        guard hex.count.isMultiple(of: 2) else { return nil }
        var bytes = [UInt8]()
        var index = hex.startIndex
        while index < hex.endIndex {
            let next = hex.index(index, offsetBy: 2)
            guard let byte = UInt8(hex[index..<next], radix: 16) else { return nil }
            bytes.append(byte)
            index = next
        }
        self.init(bytes)
    }
}

@Test func mirrorAcceptsOnlyStableIdCharacters() {
    #expect(NotificationKey.Mirror.isValid(nodeId: "n3BwZ18yBM11CNTRL"))
    #expect(NotificationKey.Mirror.isValid(nodeId: "nMAC-1_a"))
    for bad in ["", "../keys", "a/b", "a.key", "n\u{0}x", "é", String(repeating: "n", count: 65)] {
        #expect(!NotificationKey.Mirror.isValid(nodeId: bad), "\(bad)")
    }
}
