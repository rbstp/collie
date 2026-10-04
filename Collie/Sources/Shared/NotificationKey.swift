import CryptoKit
import Foundation
import Security

// Compiled into the app, ColliePush and CollieWidgets: the key never leaves the shared keychain
// group and its App Group mirror except in `push.register` over the tailnet session, and is
// never logged.

/// The per-Mac key collied seals the alert context with, so Apple only sees the plaintext fallback.
enum NotificationKey {
    static let service = "dev.rbstp.collie.notify"

    /// `$(AppIdentifierPrefix)dev.rbstp.collie.shared`, expanded into Info.plist from the same
    /// build setting as the keychain-access-groups entitlement.
    static var accessGroup: String? {
        Bundle.main.object(forInfoDictionaryKey: "CollieKeychainGroup") as? String
    }

    /// The Keychain copy, else the App Group mirror (the only one a Live Activity can read).
    static func load(nodeId: String) -> SymmetricKey? {
        keychainKey(nodeId: nodeId) ?? Mirror.read(nodeId: nodeId)
    }

    private static func keychainKey(nodeId: String) -> SymmetricKey? {
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
        if let key = keychainKey(nodeId: nodeId) {
            if Mirror.read(nodeId: nodeId).map(bytes) != bytes(key) { try Mirror.write(key, nodeId: nodeId) }
            return key
        }
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
        try Mirror.write(key, nodeId: nodeId)
    }

    static func delete(nodeId: String) {
        SecItemDelete(baseQuery(nodeId: nodeId) as CFDictionary)
        Mirror.delete(nodeId: nodeId)
    }

    private static func bytes(_ key: SymmetricKey) -> Data {
        key.withUnsafeBytes { Data($0) }
    }

    /// A Live Activity renders in a process with no Keychain (errSecNotAvailable), so each key is
    /// also kept in the App Group container: readable only by collie's own targets, encrypted by
    /// iOS until the first unlock after boot, 0600, and excluded from backups like the
    /// ThisDeviceOnly Keychain item.
    enum Mirror {
        static let directory = "notify-keys"

        /// Node ids name files, so only Tailscale's StableID alphabet is accepted.
        static func isValid(nodeId: String) -> Bool {
            !nodeId.isEmpty && nodeId.count <= 64
                && nodeId.unicodeScalars.allSatisfy { $0.isASCII && (CharacterSet.alphanumerics.contains($0) || $0 == "-" || $0 == "_") }
        }

        static func url(nodeId: String) -> URL? {
            guard isValid(nodeId: nodeId), let base = AppGroup.container else { return nil }
            return base.appending(path: directory, directoryHint: .isDirectory).appending(path: "\(nodeId).key")
        }

        static func read(nodeId: String) -> SymmetricKey? {
            guard let url = url(nodeId: nodeId), let data = try? Data(contentsOf: url), data.count == 32 else { return nil }
            return SymmetricKey(data: data)
        }

        static func write(_ key: SymmetricKey, nodeId: String) throws {
            guard let url = url(nodeId: nodeId) else { throw CocoaError(.fileWriteInvalidFileName) }
            var dir = url.deletingLastPathComponent()
            try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
            var noBackup = URLResourceValues()
            noBackup.isExcludedFromBackup = true
            try dir.setResourceValues(noBackup)
            try bytes(key).write(to: url, options: [.atomic, .completeFileProtectionUntilFirstUserAuthentication])
            try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: url.path)
            var file = url
            try file.setResourceValues(noBackup)
        }

        static func delete(nodeId: String) {
            guard let url = url(nodeId: nodeId) else { return }
            try? FileManager.default.removeItem(at: url)
        }
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
