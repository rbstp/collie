import CryptoTokenKit
import Foundation
import Security
import os

/// One generic password in the data protection keychain, holding a key's blob.
// @unchecked: accessible is one of the immutable kSecAttrAccessible constants.
struct KeychainItem: @unchecked Sendable {
    let service: String
    let account: String
    let accessible: CFString

    /// `$(AppIdentifierPrefix)dev.rbstp.collie`, the group the app alone holds, through its
    /// application identifier. A query that names no group writes to the first
    /// keychain-access-groups entry, the one ColliePush and CollieWidgets hold too, so the name
    /// is never optional.
    static let accessGroup = Bundle.main.object(forInfoDictionaryKey: "ColliePrivateKeychainGroup") as! String

    var storedService: String {
        #if DEBUG && targetEnvironment(simulator)
        if SimulatorVM.enabled { return "dev.rbstp.collie.vm-test." + service }
        #endif
        return service
    }

    var query: [String: Any] {
        [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: storedService,
            kSecAttrAccount as String: account,
            kSecAttrAccessGroup as String: Self.accessGroup,
            kSecUseDataProtectionKeychain as String: true,
        ]
    }

    var readQuery: [String: Any] {
        var query = query
        query[kSecReturnData as String] = true
        query[kSecMatchLimit as String] = kSecMatchLimitOne
        return query
    }

    func addQuery(_ data: Data) -> [String: Any] {
        var item = query
        item[kSecAttrAccessible as String] = accessible
        item[kSecValueData as String] = data
        return item
    }

    /// nil only when no item is stored. A caller makes a new key on nil, so every other failure
    /// (the Keychain is locked) throws: no key is ever made over one this read could not reach.
    func read() throws -> Data? {
        var result: CFTypeRef?
        let status = SecItemCopyMatching(readQuery as CFDictionary, &result)
        if status == errSecItemNotFound { return nil }
        guard status == errSecSuccess, let data = result as? Data else { throw KeychainError(status: status) }
        return data
    }

    /// Replaces any stored blob in place, never by a delete and an add: an add that failed (the
    /// phone locked in between) would leave no copy.
    func write(_ data: Data) throws {
        var status = SecItemUpdate(query as CFDictionary, [kSecValueData as String: data] as CFDictionary)
        if status == errSecItemNotFound { status = SecItemAdd(addQuery(data) as CFDictionary, nil) }
        guard status == errSecSuccess else { throw KeychainError(status: status) }
    }
}

/// A Secure Enclave key rebuilt from its stored blob. A new key costs a pairing with every
/// machine, so one is made only when no blob is stored or the enclave calls the blob corrupt.
enum EnclaveBlob {
    private static let log = Logger(subsystem: "dev.rbstp.collie", category: "keys")

    /// The blob is kept. The description names the cause's domain and code, which its
    /// localizedDescription drops: they say whether `isDead` should learn the error.
    struct Kept: LocalizedError {
        let cause: any Error
        var errorDescription: String? { "The stored key could not be used: \(cause)" }
    }

    /// Apple documents no error of the CryptoKit initializers, so this rests on reports and
    /// keeps a key in doubt. macOS's CryptoKit reaches the enclave through CryptoTokenKit. In
    /// Apple Developer Forums thread 748565, data that was no enclave blob gave corruptedData
    /// with no AKSError, most likely from this initializer; in threads 124196 and 682162 the key
    /// store's own failures, a locked phone among them, gave the same code with an AKSError
    /// when signing. Every other error, and any error while the phone is locked, may pass: it
    /// says nothing about the blob.
    static func isDead(_ error: any Error, unlocked: Bool) -> Bool {
        let error = error as NSError
        return unlocked && error.domain == TKErrorDomain && error.code == TKError.Code.corruptedData.rawValue
            && error.userInfo["AKSError"] == nil
    }

    static func key<Key>(
        stored: Data?, unlocked: Bool, rebuild: (Data) throws -> Key, make: () throws -> (key: Key, blob: Data),
        write: (Data) throws -> Void
    ) throws -> Key {
        if let stored {
            do { return try rebuild(stored) } catch {
                guard isDead(error, unlocked: unlocked) else {
                    log.error("stored key kept: \(String(describing: error), privacy: .public)")
                    throw Kept(cause: error)
                }
            }
        }
        let made = try make()
        try write(made.blob)
        return made.key
    }

    static var keys: [KeychainItem] { [Identity.keychain, TerminalKey.keychain] }

    /// The reset the startup screen offers, only ever after the owner's confirmation: every
    /// machine refuses the new key until the phone is revoked there and paired again. The error
    /// on that screen is as old as the process, which may have started while the phone was
    /// locked, so the keys are deleted only when the rebuild still fails now.
    static func reset(
        _ items: [KeychainItem] = keys, delete: (CFDictionary) -> OSStatus = SecItemDelete, rebuild: () throws -> Void
    ) {
        do { try rebuild() } catch is Kept {
            for item in items { _ = delete(item.query as CFDictionary) }
        } catch {}
    }
}
