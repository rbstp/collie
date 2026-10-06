import CryptoKit
import Foundation
import LocalAuthentication
import Security

/// Signs terminal grants, which collied checks against the public key recorded at pairing: a
/// Secure Enclave P-256 key, apart from the TLS key, that signs only after Face ID or the
/// passcode. It exists only while the phone has a passcode; removing it destroys the key.
enum TerminalKey {
    private static let service = "dev.rbstp.collie.identity"

    /// nil when this phone cannot hold one (no passcode, or the phone is locked): terminals
    /// then ask to pair again. On a device there is no software fallback.
    static func publicKey() -> Data? {
        #if targetEnvironment(simulator)
        return try? softKey().publicKey.derRepresentation
        #else
        return try? enclaveKey(context: nil).publicKey.derRepresentation
        #endif
    }

    /// A fresh context per grant, never another's evaluated one, so one unlock never covers a
    /// later grant. nil when cancelled or refused: nothing is signed.
    static func sign(_ message: Data, reason: String) async -> Data? {
        #if targetEnvironment(simulator)
        let context = LAContext()
        guard (try? await context.evaluatePolicy(.deviceOwnerAuthentication, localizedReason: reason)) == true
        else { return nil }
        return try? softKey().signature(for: message).derRepresentation
        #else
        // The signature waits for Face ID: off the main actor.
        return await Task.detached {
            let context = LAContext()
            context.localizedReason = reason
            return try? enclaveKey(context: context).signature(for: message).derRepresentation
        }.value
        #endif
    }

    #if targetEnvironment(simulator)
    private static func softKey() throws -> P256.Signing.PrivateKey {
        if let stored = try stored() { return try P256.Signing.PrivateKey(rawRepresentation: stored) }
        let key = P256.Signing.PrivateKey()
        try store(key.rawRepresentation, accessible: kSecAttrAccessibleWhenUnlockedThisDeviceOnly)
        return key
    }
    #else
    private static func enclaveKey(context: LAContext?) throws -> SecureEnclave.P256.Signing.PrivateKey {
        guard SecureEnclave.isAvailable else { throw CocoaError(.featureUnsupported) }
        if let stored = try stored(),
            let key = try? SecureEnclave.P256.Signing.PrivateKey(dataRepresentation: stored, authenticationContext: context)
        {
            return key
        }
        var error: Unmanaged<CFError>?
        guard let access = SecAccessControlCreateWithFlags(
            nil, kSecAttrAccessibleWhenPasscodeSetThisDeviceOnly, [.privateKeyUsage, .userPresence], &error)
        else { throw error!.takeRetainedValue() as Error }
        let key = try SecureEnclave.P256.Signing.PrivateKey(accessControl: access, authenticationContext: context)
        try store(key.dataRepresentation, accessible: kSecAttrAccessibleWhenPasscodeSetThisDeviceOnly)
        return key
    }
    #endif

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

    private static func store(_ data: Data, accessible: CFString) throws {
        SecItemDelete(baseQuery() as CFDictionary)
        var item = baseQuery()
        item[kSecAttrAccessible as String] = accessible
        item[kSecValueData as String] = data
        let status = SecItemAdd(item as CFDictionary, nil)
        guard status == errSecSuccess else { throw KeychainError(status: status) }
    }

    private static func baseQuery() -> [String: Any] {
        [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: "terminal",
            kSecUseDataProtectionKeychain as String: true,
        ]
    }
}

/// Face ID or the passcode, then a signature by the terminal key; a seam for tests.
protocol TerminalUnlocker: Sendable {
    /// Without a passcode the phone has no terminal key, and pairing again cannot give it one.
    var passcodeSet: Bool { get }
    func sign(_ message: Data, reason: String) async -> Data?
}

struct SecureEnclaveUnlocker: TerminalUnlocker {
    var passcodeSet: Bool { LAContext().canEvaluatePolicy(.deviceOwnerAuthentication, error: nil) }

    func sign(_ message: Data, reason: String) async -> Data? {
        await TerminalKey.sign(message, reason: reason)
    }
}
