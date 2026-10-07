import Foundation
import Security

/// One generic password in the data protection keychain, holding a key's blob.
struct KeychainItem {
    let service: String
    let account: String

    var query: [String: Any] {
        [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
            kSecUseDataProtectionKeychain as String: true,
        ]
    }

    var readQuery: [String: Any] {
        var query = query
        query[kSecReturnData as String] = true
        query[kSecMatchLimit as String] = kSecMatchLimitOne
        return query
    }

    func addQuery(_ data: Data, accessible: CFString) -> [String: Any] {
        var item = query
        item[kSecAttrAccessible as String] = accessible
        item[kSecValueData as String] = data
        return item
    }

    func read() throws -> Data? {
        var result: CFTypeRef?
        let status = SecItemCopyMatching(readQuery as CFDictionary, &result)
        if status == errSecItemNotFound { return nil }
        guard status == errSecSuccess, let data = result as? Data else { throw KeychainError(status: status) }
        return data
    }

    /// Replaces any stored blob.
    func write(_ data: Data, accessible: CFString) throws {
        SecItemDelete(query as CFDictionary)
        let status = SecItemAdd(addQuery(data, accessible: accessible) as CFDictionary, nil)
        guard status == errSecSuccess else { throw KeychainError(status: status) }
    }
}
