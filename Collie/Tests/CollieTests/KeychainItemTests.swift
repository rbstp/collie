import CryptoKit
import Foundation
import Security
import Testing

@testable import Collie

/// The query builders as they were before KeychainItem (6267cc0). Any drift would lose an
/// existing pairing or terminal key, so these fixtures must never be edited. #805 added the
/// app's own access group to every query (`grouped`) and moves the items an older build wrote.
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

private func grouped(_ query: [String: Any]) -> [String: Any] {
    var query = query
    query[kSecAttrAccessGroup as String] = KeychainItem.accessGroup
    return query
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
        expectSame(item.query, grouped(Before.identityBase()))
        expectSame(item.readQuery, grouped(Before.read(Before.identityBase())))
        expectSame(
            item.addQuery(blob),
            grouped(Before.add(Before.identityBase(), blob, accessible: kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly))
        )
    }

    @Test func terminalKeyQueriesUnchanged() {
        let item = TerminalKey.keychain
        expectSame(item.query, grouped(Before.terminalBase()))
        expectSame(item.readQuery, grouped(Before.read(Before.terminalBase())))
        #if targetEnvironment(simulator)
        let accessible = kSecAttrAccessibleWhenUnlockedThisDeviceOnly
        #else
        let accessible = kSecAttrAccessibleWhenPasscodeSetThisDeviceOnly
        #endif
        expectSame(item.addQuery(blob), grouped(Before.add(Before.terminalBase(), blob, accessible: accessible)))
    }
}

/// #805: the queries below run on every host, an unsigned one included.
struct KeychainGroupQueryTests {
    @Test func relayQueryNamesTheAppsOwnGroup() throws {
        let query = RelayCredentialStore(marker: URL(filePath: "/dev/null")).query
        #expect(query[kSecAttrAccessGroup as String] as? String == KeychainItem.accessGroup)
        #expect(query[kSecAttrService as String] as? String == "dev.rbstp.collie.relay")
        #expect(query[kSecAttrAccount as String] as? String == "installation")
    }

    /// A move or a delete without the shared group would match the app's own copy.
    @Test func moveNamesTheSharedGroupAndOneItem() throws {
        let shared = try #require(NotificationKey.accessGroup)
        #expect(shared != KeychainItem.accessGroup)
        let items = KeychainItem.movedItems
        #expect(items.map(\.service) == ["dev.rbstp.collie.identity", "dev.rbstp.collie.identity", "dev.rbstp.collie.relay"])
        #expect(items.map(\.account) == ["tls", "terminal", "installation"])
        for (service, account) in items {
            expectSame(
                KeychainItem.sharedQuery(service: service, account: account),
                [
                    kSecClass as String: kSecClassGenericPassword,
                    kSecAttrService as String: service,
                    kSecAttrAccount as String: account,
                    kSecAttrAccessGroup as String: shared,
                    kSecUseDataProtectionKeychain as String: true,
                ])
        }
        expectSame(KeychainItem.moveAttributes, [kSecAttrAccessGroup as String: KeychainItem.accessGroup])
    }
}

private func stored(_ service: String, _ account: String, in group: String? = nil) -> [String: Any] {
    var query: [String: Any] = [
        kSecClass as String: kSecClassGenericPassword,
        kSecAttrService as String: service,
        kSecAttrAccount as String: account,
        kSecUseDataProtectionKeychain as String: true,
    ]
    if let group { query[kSecAttrAccessGroup as String] = group }
    return query
}

/// An unsigned simulator build has no entitlements, and every Keychain call there fails with
/// errSecMissingEntitlement, whatever group it names. Never on a device: these tests replace
/// and delete the relay state.
private var keychainTestable: Bool {
    #if targetEnvironment(simulator)
    SecItemCopyMatching(stored("dev.rbstp.collie.test", "entitled") as CFDictionary, nil) != errSecMissingEntitlement
    #else
    false
    #endif
}

/// Every stored copy of the item, in whatever group the app can reach, as a query without a
/// group finds them: the query of a build up to 0.2.3.
private func copies(_ service: String, _ account: String) -> [String: Data] {
    var query = stored(service, account)
    query[kSecReturnAttributes as String] = true
    query[kSecReturnData as String] = true
    query[kSecMatchLimit as String] = kSecMatchLimitAll
    var result: CFTypeRef?
    SecItemCopyMatching(query as CFDictionary, &result)
    var copies: [String: Data] = [:]
    for item in (result as? [[String: Any]]) ?? [] {
        if let group = item[kSecAttrAccessGroup as String] as? String { copies[group] = item[kSecValueData as String] as? Data }
    }
    return copies
}

/// #805: the extensions hold the shared group, so the TLS key, the terminal key and the relay
/// state are kept out of it and moved out of it, and the notification keys stay in it.
@Suite(.serialized, .enabled(if: keychainTestable))
struct KeychainGroupTests {
    let own = KeychainItem.accessGroup
    let shared: String
    let old = Data([0x01]), new = Data([0x02])
    let locked: (String, String) -> OSStatus = { _, _ in errSecInteractionNotAllowed }

    init() throws {
        shared = try #require(NotificationKey.accessGroup)
        try #require(shared != own)
    }

    private func item(_ accessible: CFString = kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly) -> KeychainItem {
        KeychainItem(service: "dev.rbstp.collie.test." + UUID().uuidString, account: "item", accessible: accessible)
    }

    private func add(_ service: String, _ account: String, in group: String, _ data: Data, _ accessible: CFString = kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly) throws {
        var add = stored(service, account, in: group)
        add[kSecAttrAccessible as String] = accessible
        add[kSecValueData as String] = data
        let status = SecItemAdd(add as CFDictionary, nil)
        try #require(status == errSecSuccess)
    }

    private func remove(_ service: String, _ account: String) {
        SecItemDelete(stored(service, account) as CFDictionary)
    }

    private func accessible(_ service: String, _ account: String) -> String? {
        var query = stored(service, account, in: own)
        query[kSecReturnAttributes as String] = true
        var result: CFTypeRef?
        SecItemCopyMatching(query as CFDictionary, &result)
        return (result as? [String: Any])?[kSecAttrAccessible as String] as? String
    }

    @Test func itemIsWrittenToTheAppsOwnGroupAndFoundWithoutAGroup() throws {
        let item = item()
        defer { remove(item.service, item.account) }
        try item.write(new)
        #expect(copies(item.service, item.account) == [own: new])
    }

    @Test func writeRemovesTheSharedCopy() throws {
        let item = item()
        defer { remove(item.service, item.account) }
        try add(item.service, item.account, in: shared, old)
        try item.write(new)
        #expect(copies(item.service, item.account) == [own: new])
    }

    @Test(arguments: [kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly as String, kSecAttrAccessibleWhenUnlockedThisDeviceOnly as String])
    func readMovesTheSharedCopy(accessible: String) throws {
        let item = item(accessible as CFString)
        defer { remove(item.service, item.account) }
        try add(item.service, item.account, in: shared, old, accessible as CFString)
        #expect(try item.read() == old)
        #expect(copies(item.service, item.account) == [own: old])
        #expect(self.accessible(item.service, item.account) == accessible)
    }

    @Test func readFindsNothingToMove() throws {
        let item = item()
        #expect(try item.read() == nil)
        #expect(copies(item.service, item.account).isEmpty)
    }

    /// A nil here would have Identity and TerminalKey make a new key over the old one.
    @Test func readThrowsWhileTheSharedCopyCannotBeMoved() throws {
        let item = item()
        defer { remove(item.service, item.account) }
        try add(item.service, item.account, in: shared, old)
        let error = #expect(throws: KeychainError.self) { try item.read(move: locked) }
        #expect(error?.status == errSecInteractionNotAllowed)
        #expect(copies(item.service, item.account) == [shared: old])
        #expect(try item.read() == old)
    }

    @Test func relayStateIsMovedAndLoadedUnderItsMarker() throws {
        let (service, account) = (RelayCredentialStore.service, RelayCredentialStore.account)
        defer { remove(service, account) }
        remove(service, account)
        let state = RelayEnrollmentState()
        let data = try JSONEncoder().encode(state)
        let store = RelayCredentialStore(marker: try makeTemporaryDirectory().appending(path: "relay-installation"))
        try Data(state.installationId.utf8).write(to: store.marker)
        try add(service, account, in: shared, data)

        #expect(try store.load().installationId == state.installationId)
        #expect(copies(service, account) == [own: data])

        try add(service, account, in: shared, old)
        try store.save(state)
        #expect(Array(copies(service, account).keys) == [own])
    }

    @Test func sweepMovesEachItemAndLeavesNotificationKeys() throws {
        let items = [item(), item(), item()]
        let nodeId = "nTEST" + UUID().uuidString.replacingOccurrences(of: "-", with: "")
        defer {
            items.forEach { remove($0.service, $0.account) }
            NotificationKey.delete(nodeId: nodeId)
        }
        for item in items { try add(item.service, item.account, in: shared, old) }
        try NotificationKey.store(SymmetricKey(size: .bits256), nodeId: nodeId)
        let key = copies(NotificationKey.service, nodeId)
        #expect(Array(key.keys) == [shared])

        KeychainItem.moveSharedItems(items.map { ($0.service, $0.account) })

        for item in items { #expect(copies(item.service, item.account) == [own: old]) }
        #expect(copies(NotificationKey.service, nodeId) == key)
    }

    @Test func sweepKeepsTheAppsOwnCopyOfTwo() throws {
        let item = item()
        defer { remove(item.service, item.account) }
        try add(item.service, item.account, in: own, new)
        try add(item.service, item.account, in: shared, old)

        KeychainItem.moveSharedItems([(item.service, item.account)])

        #expect(copies(item.service, item.account) == [own: new])
    }
}
