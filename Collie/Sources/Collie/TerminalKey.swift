import CryptoKit
import Foundation
import LocalAuthentication
import Security

/// Signs terminal grants, which collied checks against the public key recorded at pairing: a
/// Secure Enclave P-256 key, apart from the TLS key, that signs only after Face ID or the
/// passcode. It exists only while the phone has a passcode; removing it destroys the key.
enum TerminalKey {
    #if targetEnvironment(simulator)
    static let keychain = KeychainItem(
        service: "dev.rbstp.collie.identity", account: "terminal", accessible: kSecAttrAccessibleWhenUnlockedThisDeviceOnly)
    #else
    static let keychain = KeychainItem(
        service: "dev.rbstp.collie.identity", account: "terminal", accessible: kSecAttrAccessibleWhenPasscodeSetThisDeviceOnly)
    #endif

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
        if let stored = try keychain.read() { return try P256.Signing.PrivateKey(rawRepresentation: stored) }
        let key = P256.Signing.PrivateKey()
        try keychain.write(key.rawRepresentation)
        return key
    }
    #else
    private static func enclaveKey(context: LAContext?) throws -> SecureEnclave.P256.Signing.PrivateKey {
        guard SecureEnclave.isAvailable else { throw CocoaError(.featureUnsupported) }
        if let stored = try keychain.read(),
            let key = try? SecureEnclave.P256.Signing.PrivateKey(dataRepresentation: stored, authenticationContext: context)
        {
            return key
        }
        var error: Unmanaged<CFError>?
        guard let access = SecAccessControlCreateWithFlags(
            nil, kSecAttrAccessibleWhenPasscodeSetThisDeviceOnly, [.privateKeyUsage, .userPresence], &error)
        else { throw error!.takeRetainedValue() as Error }
        let key = try SecureEnclave.P256.Signing.PrivateKey(accessControl: access, authenticationContext: context)
        try keychain.write(key.dataRepresentation)
        return key
    }
    #endif
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
