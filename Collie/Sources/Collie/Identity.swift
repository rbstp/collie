import CollieCore
import CryptoKit
import Foundation
import Security

/// The phone's TLS key, which collied pins at pairing: a Secure Enclave P-256 key that never
/// leaves this device. Usable after the first unlock, so lock-screen decisions can connect.
enum Identity {
    private static let service = "dev.rbstp.collie.identity"

    static func load() throws -> (publicKey: Data, signer: IdentitySigner) {
        #if targetEnvironment(simulator)
        let key = try stored().map { try P256.Signing.PrivateKey(rawRepresentation: $0) } ?? {
            let key = P256.Signing.PrivateKey()
            try store(key.rawRepresentation)
            return key
        }()
        return (key.publicKey.derRepresentation, Signer { try key.signature(for: $0).derRepresentation })
        #else
        guard SecureEnclave.isAvailable else { throw CocoaError(.featureUnsupported) }
        let key = try stored().map { try SecureEnclave.P256.Signing.PrivateKey(dataRepresentation: $0) } ?? {
            var error: Unmanaged<CFError>?
            guard let access = SecAccessControlCreateWithFlags(
                nil, kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly, .privateKeyUsage, &error)
            else { throw error!.takeRetainedValue() as Error }
            let key = try SecureEnclave.P256.Signing.PrivateKey(accessControl: access)
            try store(key.dataRepresentation)
            return key
        }()
        return (key.publicKey.derRepresentation, Signer { try key.signature(for: $0).derRepresentation })
        #endif
    }

    /// On a device this is the Secure Enclave's wrapped key, which only that enclave can use.
    private static func stored() throws -> Data? {
        var query = baseQuery()
        query[kSecReturnData as String] = true
        query[kSecMatchLimit as String] = kSecMatchLimitOne
        var result: CFTypeRef?
        let status = SecItemCopyMatching(query as CFDictionary, &result)
        if status == errSecItemNotFound { return nil }
        guard status == errSecSuccess, let data = result as? Data else { throw KeychainError(status: status) }
        return data
    }

    private static func store(_ data: Data) throws {
        var item = baseQuery()
        item[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
        item[kSecValueData as String] = data
        let status = SecItemAdd(item as CFDictionary, nil)
        guard status == errSecSuccess else { throw KeychainError(status: status) }
    }

    private static func baseQuery() -> [String: Any] {
        [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: "tls",
            kSecUseDataProtectionKeychain as String: true,
        ]
    }

    private final class Signer: IdentitySigner {
        private let signature: @Sendable (Data) throws -> Data

        init(_ signature: @escaping @Sendable (Data) throws -> Data) {
            self.signature = signature
        }

        func sign(message: Data) -> Data? {
            try? signature(message)
        }
    }
}
