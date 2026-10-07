import CollieCore
import CryptoKit
import Foundation
import Security

/// The phone's TLS key, which collied pins at pairing: a Secure Enclave P-256 key that never
/// leaves this device. Usable after the first unlock, so lock-screen decisions can connect.
enum Identity {
    /// On a device it holds the Secure Enclave's wrapped key, which only that enclave can use.
    static let keychain = KeychainItem(service: "dev.rbstp.collie.identity", account: "tls")

    static func load() throws -> (publicKey: Data, signer: IdentitySigner) {
        #if targetEnvironment(simulator)
        let key = try keychain.read().map { try P256.Signing.PrivateKey(rawRepresentation: $0) } ?? {
            let key = P256.Signing.PrivateKey()
            try keychain.write(key.rawRepresentation, accessible: kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly)
            return key
        }()
        return (key.publicKey.derRepresentation, Signer { try key.signature(for: $0).derRepresentation })
        #else
        guard SecureEnclave.isAvailable else { throw CocoaError(.featureUnsupported) }
        // A blob this enclave can no longer use is replaced: machines then refuse the new key
        // until the phone is revoked there and paired again.
        let key = try keychain.read().flatMap { try? SecureEnclave.P256.Signing.PrivateKey(dataRepresentation: $0) } ?? {
            var error: Unmanaged<CFError>?
            guard let access = SecAccessControlCreateWithFlags(
                nil, kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly, .privateKeyUsage, &error)
            else { throw error!.takeRetainedValue() as Error }
            let key = try SecureEnclave.P256.Signing.PrivateKey(accessControl: access)
            try keychain.write(key.dataRepresentation, accessible: kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly)
            return key
        }()
        return (key.publicKey.derRepresentation, Signer { try key.signature(for: $0).derRepresentation })
        #endif
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
