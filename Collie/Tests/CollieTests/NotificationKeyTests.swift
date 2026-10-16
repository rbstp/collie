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

@Test func displayEnvelopeSharedVectorsRejectEveryWrongBinding() throws {
    let url = URL(filePath: #filePath).deletingLastPathComponent().appending(path: "../../../docs/protocol/push-display-vectors.json")
    let fixture = try #require(try JSONSerialization.jsonObject(with: Data(contentsOf: url)) as? [String: Any])
    for vector in try #require(fixture["vectors"] as? [[String: Any]]) {
        let enc = try #require(vector["enc"] as? String)
        let kind = UInt8(try #require(vector["kind"] as? Int))
        let node = try #require(vector["node_id"] as? String)
        let scope = try #require(vector["scope"] as? String)
        let event = try #require(vector["event_id"] as? String)
        let expected = try #require(vector["plaintext"] as? String)
        let opened = try #require(PushEnvelope.open(enc, kind: kind, nodeId: node, scope: scope, eventId: event, key: vectorKey, as: TestDisplay.self))
        #expect(opened == (try JSONDecoder().decode(TestDisplay.self, from: Data(expected.utf8))))
        for (otherKind, otherNode, otherScope, otherEvent) in [(kind % 3 + 1, node, scope, event), (kind, "nOther", scope, event), (kind, node, "another-activity", event), (kind, node, scope, "another-event")] {
            #expect(PushEnvelope.open(enc, kind: otherKind, nodeId: otherNode, scope: otherScope, eventId: otherEvent, key: vectorKey, as: TestDisplay.self) == nil)
        }
        #expect(PushEnvelope.open(enc, kind: kind, nodeId: node, scope: scope, eventId: event, key: SymmetricKey(size: .bits256), as: TestDisplay.self) == nil)
        #expect(PushEnvelope.header(enc + "=") == nil)
        #expect(PushEnvelope.header(String(repeating: "A", count: 3801)) == nil)
        var tampered = Array(enc.utf8)
        tampered[tampered.count - 3] = tampered[tampered.count - 3] == 65 ? 66 : 65
        #expect(PushEnvelope.open(String(decoding: tampered, as: UTF8.self), kind: kind, nodeId: node, scope: scope, eventId: event, key: vectorKey, as: TestDisplay.self) == nil)
    }
}

private struct TestDisplay: Decodable, Equatable {
    let v: Int
    let title: String?
    let body: String?
    let terminalId: String?
    let approvalId: String?
    let offersApproval: Bool?
    let machineLabel: String?
    let state: TestDisplayState?
}

private struct TestDisplayState: Decodable, Equatable {
    let status: String
    let statusSince: Double
    let title: String
    let kind: String?
    let workspace: String?
    let approvals: Int
    let approvalId: String?
    let enc: String?
}

@Test func alertDisplayAuthenticatesTheApprovalBeforeOfferingControls() throws {
    let url = URL(filePath: #filePath).deletingLastPathComponent().appending(path: "../../../docs/protocol/push-display-vectors.json")
    let fixture = try #require(try JSONSerialization.jsonObject(with: Data(contentsOf: url)) as? [String: Any])
    let vectors = try #require(fixture["vectors"] as? [[String: Any]])
    var payload = try #require(vectors.first?["payload"] as? [String: Any])
    let display = try #require(PushDisplay.open(userInfo: payload, key: vectorKey))
    #expect(display.title == "api-fixer")
    #expect(display.body == "Bash: echo hi")
    #expect(display.terminalId == "term_1")
    #expect(display.approvalId == "apr_test")
    #expect(display.offersApproval)
    payload["approval_id"] = "apr_other"
    #expect(PushDisplay.open(userInfo: payload, key: vectorKey) == nil)
}

@Test func displayEnvelopeBoundsUnicodeEscapingAndInitialActivityStorage() throws {
    struct AttributesDisplay: Encodable { let v = 1; let machineLabel: String; let title: String }
    struct StateDisplay: Encodable { let v = 1; let state: State }
    struct State: Encodable {
        let status = "blocked"
        let statusSince = 812721600
        let title: String
        let workspace: String
        let approvals = 1
    }
    for text in [String(repeating: "😀", count: 64), String(repeating: "\"", count: 64), String(repeating: "\u{1}", count: 64)] {
        let binding = String(repeating: "A", count: 64)
        let node = String(repeating: "n", count: 64)
        let attrsEnc = try #require(PushEnvelope.seal(AttributesDisplay(machineLabel: text, title: text), kind: 3, nodeId: node, scope: binding, key: vectorKey))
        let stateEnc = try #require(PushEnvelope.seal(StateDisplay(state: State(title: text, workspace: text)), kind: 2, nodeId: node, scope: binding, key: vectorKey))
        let attrs = try JSONSerialization.data(withJSONObject: ["machineId": String(repeating: "m", count: 64), "terminalId": String(repeating: "t", count: 64), "nodeId": node, "bindingId": binding, "enc": attrsEnc])
        let state = try JSONSerialization.data(withJSONObject: ["v": 1, "enc": stateEnc])
        #expect(attrs.count + state.count <= 4000)
        #expect(attrsEnc.utf8.count <= 3800)
        #expect(stateEnc.utf8.count <= 3800)
        let decodedAttrs = try JSONDecoder().decode(AgentActivityAttributes.self, from: attrs)
        let decodedState = try JSONDecoder().decode(AgentActivityAttributes.ContentState.self, from: state)
        #expect(try JSONEncoder().encode(decodedAttrs).count + JSONEncoder().encode(decodedState).count <= 4000)
    }
    #expect(PushEnvelope.seal(AttributesDisplay(machineLabel: "x", title: String(repeating: "😀", count: 4096)), kind: 3, nodeId: "nMAC", scope: "binding", key: vectorKey) == nil)
}
