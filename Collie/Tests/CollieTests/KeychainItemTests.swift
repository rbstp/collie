import Foundation
import Security
import Testing

@testable import Collie

/// The query builders as they were before KeychainItem (6267cc0). Any drift would lose an
/// existing pairing or terminal key, so these fixtures must never be edited.
private enum Before {
    static func identityBase() -> [String: Any] {
        [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: "dev.rbstp.collie.identity",
            kSecAttrAccount as String: "tls",
            kSecUseDataProtectionKeychain as String: true,
        ]
    }

    static func terminalBase() -> [String: Any] {
        [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: "dev.rbstp.collie.identity",
            kSecAttrAccount as String: "terminal",
            kSecUseDataProtectionKeychain as String: true,
        ]
    }

    static func read(_ base: [String: Any]) -> [String: Any] {
        var query = base
        query[kSecReturnData as String] = true
        query[kSecMatchLimit as String] = kSecMatchLimitOne
        return query
    }

    static func add(_ base: [String: Any], _ data: Data, accessible: CFString) -> [String: Any] {
        var item = base
        item[kSecAttrAccessible as String] = accessible
        item[kSecValueData as String] = data
        return item
    }
}

private func expectSame(_ actual: [String: Any], _ expected: [String: Any]) {
    #expect(Set(actual.keys) == Set(expected.keys))
    for (key, value) in expected {
        guard let got = actual[key] else { continue }
        #expect(CFGetTypeID(got as CFTypeRef) == CFGetTypeID(value as CFTypeRef), "\(key)")
        #expect(CFEqual(got as CFTypeRef, value as CFTypeRef), "\(key)")
    }
}

struct KeychainItemTests {
    let blob = Data([0x01, 0x02, 0x03])

    @Test func identityQueriesUnchanged() {
        let item = Identity.keychain
        expectSame(item.query, Before.identityBase())
        expectSame(item.readQuery, Before.read(Before.identityBase()))
        expectSame(
            item.addQuery(blob, accessible: kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly),
            Before.add(Before.identityBase(), blob, accessible: kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly)
        )
    }

    @Test func terminalKeyQueriesUnchanged() {
        let item = TerminalKey.keychain
        expectSame(item.query, Before.terminalBase())
        expectSame(item.readQuery, Before.read(Before.terminalBase()))
        for accessible in [kSecAttrAccessibleWhenPasscodeSetThisDeviceOnly, kSecAttrAccessibleWhenUnlockedThisDeviceOnly] {
            expectSame(
                item.addQuery(blob, accessible: accessible),
                Before.add(Before.terminalBase(), blob, accessible: accessible)
            )
        }
    }
}
