import CryptoKit
import Foundation
import Security

enum PushEnvelope {
    static let maxEncoded = 3800
    private static let domain = Data("collie.push.v1\0".utf8)

    struct Header {
        let kind: UInt8
        let nodeId: String
        let scope: String
        let eventId: String
        let bytes: Data
        let combined: Data
    }

    static func base64(_ data: Data) -> String {
        data.base64EncodedString().replacingOccurrences(of: "+", with: "-").replacingOccurrences(of: "/", with: "_")
            .replacingOccurrences(of: "=", with: "")
    }

    static func header(_ enc: String) -> Header? {
        guard enc.utf8.count <= maxEncoded, !enc.isEmpty,
            enc.utf8.allSatisfy({ (65...90).contains($0) || (97...122).contains($0) || (48...57).contains($0) || $0 == 45 || $0 == 95 })
        else { return nil }
        let padded = enc.replacingOccurrences(of: "-", with: "+").replacingOccurrences(of: "_", with: "/")
            + String(repeating: "=", count: (4 - enc.utf8.count % 4) % 4)
        guard let data = Data(base64Encoded: padded), base64(data) == enc, data.count >= 48,
            data[0] == 1, (1...3).contains(data[1]), (1...64).contains(data[2]), (1...64).contains(data[3])
        else { return nil }
        let nodeEnd = 4 + Int(data[2])
        let scopeEnd = nodeEnd + Int(data[3])
        let end = scopeEnd + 16
        guard data.count >= end + 29,
            let node = String(data: data[4..<nodeEnd], encoding: .utf8), ProtocolId.valid(node, extra: "_-"),
            let scope = String(data: data[nodeEnd..<scopeEnd], encoding: .utf8), ProtocolId.valid(scope, extra: "_-")
        else { return nil }
        return Header(kind: data[1], nodeId: node, scope: scope, eventId: base64(data[scopeEnd..<end]),
            bytes: Data(data[..<end]), combined: Data(data[end...]))
    }

    static func open<T: Decodable>(
        _ enc: String, kind: UInt8, nodeId: String, scope: String, eventId: String? = nil,
        key: SymmetricKey, as type: T.Type
    ) -> T? {
        guard let header = header(enc), header.kind == kind, header.nodeId == nodeId, header.scope == scope,
            eventId == nil || eventId == header.eventId,
            let box = try? ChaChaPoly.SealedBox(combined: header.combined)
        else { return nil }
        let digest = key.withUnsafeBytes { Data(SHA256.hash(data: Data($0))) }
        guard let plain = try? ChaChaPoly.open(box, using: key, authenticating: domain + header.bytes + digest)
        else { return nil }
        return try? JSONDecoder().decode(type, from: plain)
    }

    static func seal<T: Encodable>(_ value: T, kind: UInt8, nodeId: String, scope: String, key: SymmetricKey) -> String? {
        guard (1...3).contains(kind), ProtocolId.valid(nodeId, extra: "_-"), ProtocolId.valid(scope, extra: "_-") else { return nil }
        var event = Data(count: 16)
        guard event.withUnsafeMutableBytes({ SecRandomCopyBytes(kSecRandomDefault, 16, $0.baseAddress!) }) == errSecSuccess else { return nil }
        let header = Data([1, kind, UInt8(nodeId.utf8.count), UInt8(scope.utf8.count)]) + Data(nodeId.utf8) + Data(scope.utf8) + event
        let digest = key.withUnsafeBytes { Data(SHA256.hash(data: Data($0))) }
        guard let plain = try? JSONEncoder().encode(value),
            let sealed = try? ChaChaPoly.seal(plain, using: key, authenticating: domain + header + digest)
        else { return nil }
        let enc = base64(header + sealed.combined)
        return enc.utf8.count <= maxEncoded ? enc : nil
    }
}

struct PushDisplay: Codable {
    let v: Int
    let title: String
    let body: String
    let terminalId: String
    let approvalId: String?
    let offersApproval: Bool

    static func open(userInfo: [AnyHashable: Any], key: SymmetricKey) -> Self? {
        guard let enc = userInfo["enc"] as? String, let node = userInfo["node_id"] as? String,
            let event = userInfo["event_id"] as? String, let aps = userInfo["aps"] as? [String: Any],
            let scope = aps["thread-id"] as? String,
            let display = PushEnvelope.open(enc, kind: 1, nodeId: node, scope: scope, eventId: event, key: key, as: Self.self),
            display.v == 1, ProtocolId.valid(display.terminalId, extra: "_:.-"),
            display.approvalId == userInfo["approval_id"] as? String
        else { return nil }
        return display
    }

    static func clean(_ text: String, maxLength: Int) -> String {
        var output = String.UnicodeScalarView()
        for scalar in text.unicodeScalars.prefix(maxLength)
        where scalar == "\n" || (scalar.properties.generalCategory != .control && scalar.properties.generalCategory != .format) {
            output.append(scalar)
        }
        return String(output).trimmingCharacters(in: .whitespacesAndNewlines)
    }
}
