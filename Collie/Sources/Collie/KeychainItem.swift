import Foundation
import Security

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
        [
            (Identity.keychain.storedService, Identity.keychain.account),
            (TerminalKey.keychain.storedService, TerminalKey.keychain.account),
            (RelayCredentialStore.service, RelayCredentialStore.account),
        ]
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

    /// At launch. A failure is left to the read path (the Keychain is locked before the first
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
    /// no key is ever made over an older one this read could not reach.
    func read(move: (String, String) -> OSStatus = KeychainItem.moveShared) throws -> Data? {
        var result: CFTypeRef?
        var status = SecItemCopyMatching(readQuery as CFDictionary, &result)
        if status == errSecItemNotFound {
            let moved = move(storedService, account)
            if moved == errSecItemNotFound { return nil }
            guard moved == errSecSuccess else { throw KeychainError(status: moved) }
            status = SecItemCopyMatching(readQuery as CFDictionary, &result)
        }
        guard status == errSecSuccess, let data = result as? Data else { throw KeychainError(status: status) }
        return data
    }

    /// Replaces any stored blob, and then a copy left in the shared group, which would keep the
    /// replaced key readable by the extensions.
    func write(_ data: Data) throws {
        SecItemDelete(query as CFDictionary)
        let status = SecItemAdd(addQuery(data) as CFDictionary, nil)
        guard status == errSecSuccess else { throw KeychainError(status: status) }
        SecItemDelete(Self.sharedQuery(service: storedService, account: account) as CFDictionary)
    }
}
