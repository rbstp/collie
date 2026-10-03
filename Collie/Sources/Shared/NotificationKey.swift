import CryptoKit
import Foundation
import Security

// Compiled into both the app and ColliePush: the key never leaves the shared keychain group
// except in `push.register` over the tailnet session, and is never logged.

/// The per-Mac key collied seals the alert context with, so Apple only sees the plaintext fallback.
enum NotificationKey {
    static let service = "dev.rbstp.collie.notify"

    /// `$(AppIdentifierPrefix)dev.rbstp.collie.shared`, expanded into Info.plist from the same
    /// build setting as the keychain-access-groups entitlement.
    static var accessGroup: String? {
        Bundle.main.object(forInfoDictionaryKey: "CollieKeychainGroup") as? String
    }

    static func load(nodeId: String) -> SymmetricKey? {
        var query = baseQuery(nodeId: nodeId)
        query[kSecReturnData as String] = true
        query[kSecMatchLimit as String] = kSecMatchLimitOne
        var result: CFTypeRef?
        guard SecItemCopyMatching(query as CFDictionary, &result) == errSecSuccess,
            let data = result as? Data, data.count == 32
        else { return nil }
        return SymmetricKey(data: data)
    }

    /// The stored key, or a new random one stored first.
    static func loadOrCreate(nodeId: String) throws -> SymmetricKey {
        if let key = load(nodeId: nodeId) { return key }
        try store(SymmetricKey(size: .bits256), nodeId: nodeId)
        guard let key = load(nodeId: nodeId) else { throw KeychainError(status: errSecItemNotFound) }
        return key
    }

    /// Replaces any key stored for `nodeId`.
    static func store(_ key: SymmetricKey, nodeId: String) throws {
        delete(nodeId: nodeId)
        var item = baseQuery(nodeId: nodeId)
        item[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
        item[kSecValueData as String] = key.withUnsafeBytes { Data($0) }
        let status = SecItemAdd(item as CFDictionary, nil)
        guard status == errSecSuccess else { throw KeychainError(status: status) }
    }

    static func delete(nodeId: String) {
        SecItemDelete(baseQuery(nodeId: nodeId) as CFDictionary)
    }

    private static func baseQuery(nodeId: String) -> [String: Any] {
        var query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: nodeId,
            kSecUseDataProtectionKeychain as String: true,
        ]
        if let accessGroup { query[kSecAttrAccessGroup as String] = accessGroup }
        return query
    }
}

struct KeychainError: Error, CustomStringConvertible {
    let status: OSStatus
    var description: String { "keychain error \(status)" }
}

/// The alert context sealed by collied: `enc` = base64(nonce || ciphertext || tag),
/// ChaCha20-Poly1305 with the Mac's notification key, AAD = approval id.
enum PushContext {
    static let maxLength = 600

    private struct Plaintext: Decodable {
        let v: Int
        let body: String
    }

    static func open(_ enc: String, approvalId: String, key: SymmetricKey) -> String? {
        guard let combined = Data(base64Encoded: enc),
            let box = try? ChaChaPoly.SealedBox(combined: combined),
            let plaintext = try? ChaChaPoly.open(box, using: key, authenticating: Data(approvalId.utf8)),
            let payload = try? JSONDecoder().decode(Plaintext.self, from: plaintext), payload.v == 1
        else { return nil }
        // Keeps "\n": collied joins the lines of a multi-line command with it, and gluing
        // them together would show the approver a different command.
        var body = String.UnicodeScalarView()
        for scalar in payload.body.unicodeScalars.prefix(maxLength)
        where scalar == "\n" || !scalar.properties.generalCategory.isControlOrFormat {
            body.append(scalar)
        }
        let text = String(body).trimmingCharacters(in: .whitespacesAndNewlines)
        return text.isEmpty ? nil : text
    }
}

extension Unicode.GeneralCategory {
    fileprivate var isControlOrFormat: Bool {
        self == .control || self == .format
    }
}
