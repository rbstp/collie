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

    /// The items a build up to 0.2.3 wrote without a group, so into the shared one.
    static var movedItems: [(service: String, account: String)] {
        let keys = [
            (Identity.keychain.storedService, Identity.keychain.account),
            (TerminalKey.keychain.storedService, TerminalKey.keychain.account),
        ]
        #if DEBUG && targetEnvironment(simulator)
        // The relay item has no VM name: a VM run leaves the regular one where it is.
        if SimulatorVM.enabled { return keys }
        #endif
        return keys + [(RelayCredentialStore.service, RelayCredentialStore.account)]
    }

    /// One item's copy in the group shared with the extensions. Never without the group: the
    /// move and the deletes built on it would then match the app's own copy.
    static func sharedQuery(service: String, account: String) -> [String: Any] {
        [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
            kSecAttrAccessGroup as String: NotificationKey.accessGroup!,
            kSecUseDataProtectionKeychain as String: true,
        ]
    }

    static var moveAttributes: [String: Any] { [kSecAttrAccessGroup as String: accessGroup] }

    /// Moves an item from the shared group to the app's own, data and accessibility unchanged.
    /// errSecItemNotFound when the shared group holds none, errSecDuplicateItem when the app's
    /// own group holds one already.
    static func moveShared(service: String, account: String) -> OSStatus {
        SecItemUpdate(sharedQuery(service: service, account: account) as CFDictionary, moveAttributes as CFDictionary)
    }

    /// At launch, before any other Keychain work. A failure is left to the read path (the Keychain is locked before the first
    /// unlock, and the terminal key's item whenever the phone is locked). With a copy in both
    /// groups the app's own is the one in use, and the shared one goes.
    static func moveSharedItems(_ items: [(service: String, account: String)] = movedItems) {
        for (service, account) in items {
            if moveShared(service: service, account: account) == errSecDuplicateItem {
                SecItemDelete(sharedQuery(service: service, account: account) as CFDictionary)
            }
        }
    }

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

    /// nil only when neither group holds the item. A caller makes a new key on nil, so a copy
    /// in the shared group that could not be moved (the Keychain is locked) throws instead:
    /// no key is ever made over an older one this read could not reach. The second read comes
    /// after every move, whatever it answered: another reader or the launch move may have
    /// moved the item between this read's first look and its own move, which then finds none.
    func read(move: (String, String) -> OSStatus = KeychainItem.moveShared) throws -> Data? {
        var result: CFTypeRef?
        var status = SecItemCopyMatching(readQuery as CFDictionary, &result)
        if status == errSecItemNotFound {
            let moved = move(storedService, account)
            status = SecItemCopyMatching(readQuery as CFDictionary, &result)
            if status == errSecItemNotFound {
                if moved == errSecItemNotFound { return nil }
                if moved != errSecSuccess { throw KeychainError(status: moved) }
            }
        }
        guard status == errSecSuccess, let data = result as? Data else { throw KeychainError(status: status) }
        return data
    }

    /// Replaces any stored blob in place, never by a delete and an add: an add that failed (the
    /// phone locked in between) would leave no copy. Then a copy left in the shared group goes,
    /// which would keep the replaced key readable by the extensions.
    func write(_ data: Data) throws {
        var status = SecItemUpdate(query as CFDictionary, [kSecValueData as String: data] as CFDictionary)
        if status == errSecItemNotFound { status = SecItemAdd(addQuery(data) as CFDictionary, nil) }
        guard status == errSecSuccess else { throw KeychainError(status: status) }
        SecItemDelete(Self.sharedQuery(service: storedService, account: account) as CFDictionary)
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
}
