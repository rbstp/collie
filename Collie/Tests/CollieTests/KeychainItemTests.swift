import CryptoKit
import CryptoTokenKit
import Foundation
import LocalAuthentication
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

    /// #871: the reset deletes the TLS key and the terminal key, each copy by its exact group,
    /// service and account, and no other item.
    @Test func resetNamesTheTwoKeysAndBothGroups() throws {
        let shared = try #require(NotificationKey.accessGroup)
        let keys = EnclaveBlob.keys
        #expect(keys.map(\.service) == ["dev.rbstp.collie.identity", "dev.rbstp.collie.identity"])
        #expect(keys.map(\.account) == ["tls", "terminal"])
        for key in keys {
            let queries = key.deleteQueries
            #expect(queries.count == 2)
            for (query, group) in zip(queries, [KeychainItem.accessGroup, shared]) {
                expectSame(
                    query,
                    [
                        kSecClass as String: kSecClassGenericPassword,
                        kSecAttrService as String: key.storedService,
                        kSecAttrAccount as String: key.account,
                        kSecAttrAccessGroup as String: group,
                        kSecUseDataProtectionKeychain as String: true,
                    ])
            }
        }
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

/// #871: the decision the device branches of Identity and TerminalKey share. A new key makes
/// every machine refuse the phone, so a failure that may pass must never make one.
struct EnclaveBlobTests {
    let stored = Data([0x01]), fresh = Data([0x02])
    let corrupt = NSError(
        domain: TKErrorDomain, code: TKError.Code.corruptedData.rawValue,
        userInfo: [NSLocalizedDescriptionKey: "corrupted objectID detected"])

    private func load(_ stored: Data?, unlocked: Bool = true, rebuild: (Data) throws -> String) -> (key: Result<String, any Error>, written: [Data]) {
        var written: [Data] = []
        let key = Result {
            try EnclaveBlob.key(stored: stored, unlocked: unlocked, rebuild: rebuild, make: { ("new", fresh) }, write: { written.append($0) })
        }
        return (key, written)
    }

    /// Locked too: a lock-screen decision loads the TLS key.
    @Test(arguments: [true, false])
    func goodBlobIsUsed(unlocked: Bool) throws {
        let (key, written) = load(stored, unlocked: unlocked) { $0 == stored ? "old" : "other" }
        #expect(try key.get() == "old")
        #expect(written.isEmpty)
    }

    @Test func deadBlobIsReplaced() throws {
        let (key, written) = load(stored) { _ in throw corrupt }
        #expect(try key.get() == "new")
        #expect(written == [fresh])
    }

    @Test func keyIsMadeWhenNoBlobIsStored() throws {
        let (key, written) = load(nil) { _ in "old" }
        #expect(try key.get() == "new")
        #expect(written == [fresh])
    }

    @Test func passingFailureThrowsAndWritesNothing() {
        let locked = NSError(domain: TKErrorDomain, code: TKError.Code.corruptedData.rawValue, userInfo: ["AKSError": -536_363_001])
        let failures: [(error: any Error, unlocked: Bool)] = [
            (LAError(.notInteractive), true),
            (LAError(.authenticationFailed), true),
            (LAError(.systemCancel), true),
            (NSError(domain: NSOSStatusErrorDomain, code: Int(errSecInteractionNotAllowed)), true),
            (TKError(.communicationError), true),
            (TKError(.authenticationNeeded), true),
            (TKError(.objectNotFound), true),
            (CryptoKitError.authenticationFailure, true),
            (locked, true),
            (corrupt, false),
        ]
        for (error, unlocked) in failures {
            let (key, written) = load(stored, unlocked: unlocked) { _ in throw error }
            #expect(throws: EnclaveBlob.Kept.self, "\(error)") { try key.get() }
            #expect(written.isEmpty, "\(error)")
        }
    }

    /// The startup screen offers the reset for a kept blob alone: a Keychain that cannot be read
    /// or written now, or a key that cannot be made, says nothing about the stored key.
    @Test func onlyAKeptBlobOffersTheReset() {
        let locked = KeychainError(status: errSecInteractionNotAllowed)
        let kept = #expect(throws: (any Error).self) {
            try EnclaveBlob.key(stored: stored, unlocked: true, rebuild: { _ in throw LAError(.notInteractive) }, make: { ("new", fresh) }, write: { _ in })
        }
        let unmade = #expect(throws: (any Error).self) {
            try EnclaveBlob.key(stored: nil, unlocked: true, rebuild: { _ in "old" }, make: { throw LAError(.passcodeNotSet) }, write: { _ in })
        }
        let unwritten = #expect(throws: (any Error).self) {
            try EnclaveBlob.key(stored: nil, unlocked: true, rebuild: { _ in "old" }, make: { ("new", fresh) }, write: { _ in throw locked })
        }
        #expect(kept.map(AppModel.offersKeyReset) == true)
        #expect(unmade.map(AppModel.offersKeyReset) == false)
        #expect(unwritten.map(AppModel.offersKeyReset) == false)
        #expect(!AppModel.offersKeyReset(locked))
        #expect(!AppModel.offersKeyReset(CocoaError(.featureUnsupported)))
    }

    /// The owner reads the enclave's answer on the screen that reports a kept key.
    @Test func keptKeyNamesItsCause() {
        let (key, _) = load(stored, unlocked: false) { _ in throw corrupt }
        let error = #expect(throws: EnclaveBlob.Kept.self) { try key.get() }
        let text = error.map { describe($0) } ?? ""
        #expect(text.contains("CryptoTokenKit"))
        #expect(text.contains("-3"))
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

    /// #871: a delete and an add would lose the only copy when the add fails. The stored item's
    /// accessibility tells the two apart: an update keeps it, an add sets the new item's.
    @Test func writeUpdatesTheStoredItemInPlace() throws {
        let item = item(kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly)
        defer { remove(item.service, item.account) }
        try add(item.service, item.account, in: own, old, kSecAttrAccessibleWhenUnlockedThisDeviceOnly)
        try item.write(new)
        #expect(copies(item.service, item.account) == [own: new])
        #expect(accessible(item.service, item.account) == kSecAttrAccessibleWhenUnlockedThisDeviceOnly as String)
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

    /// Another reader, or the launch move, moved the item after this read first looked: its own
    /// move then finds none, or finds both. A nil here would make a new key over the moved one.
    @Test(arguments: [errSecItemNotFound, errSecDuplicateItem])
    func readReturnsAnItemMovedMeanwhile(answer: OSStatus) throws {
        let item = item()
        defer { remove(item.service, item.account) }
        try add(item.service, item.account, in: shared, old)
        let data = try item.read { service, account in
            _ = KeychainItem.moveShared(service: service, account: account)
            return answer
        }
        #expect(data == old)
        #expect(copies(item.service, item.account) == [own: old])
    }

    @Test func readThrowsWhenAMoveLeftNoItem() throws {
        let item = item()
        let error = #expect(throws: KeychainError.self) { try item.read { _, _ in errSecSuccess } }
        #expect(error?.status == errSecItemNotFound)
        #expect(copies(item.service, item.account).isEmpty)
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

    /// #871: the reset behind the startup screen's confirmation. A shared copy left behind would
    /// be moved back by the next read, and the dead blob with it.
    @Test func resetDeletesBothCopiesOfItsKeysAndNothingElse() throws {
        let keys = [item(), item()]
        let sibling = KeychainItem(service: keys[0].service, account: "sibling", accessible: keys[0].accessible)
        let nodeId = "nTEST" + UUID().uuidString.replacingOccurrences(of: "-", with: "")
        let (relayService, relayAccount) = (RelayCredentialStore.service, RelayCredentialStore.account)
        defer {
            (keys + [sibling]).forEach { remove($0.service, $0.account) }
            NotificationKey.delete(nodeId: nodeId)
            remove(relayService, relayAccount)
        }
        remove(relayService, relayAccount)
        for item in keys + [sibling] {
            try add(item.service, item.account, in: own, new)
            try add(item.service, item.account, in: shared, old)
        }
        try add(relayService, relayAccount, in: own, old)
        try NotificationKey.store(SymmetricKey(size: .bits256), nodeId: nodeId)
        let key = copies(NotificationKey.service, nodeId)

        EnclaveBlob.reset(keys) { throw EnclaveBlob.Kept(cause: CocoaError(.featureUnsupported)) }

        for item in keys {
            #expect(copies(item.service, item.account).isEmpty)
            #expect(try item.read() == nil)
        }
        #expect(copies(sibling.service, sibling.account) == [own: new, shared: old])
        #expect(copies(relayService, relayAccount) == [own: old])
        #expect(copies(NotificationKey.service, nodeId) == key)
    }

    /// The screen's error is as old as the process: a key that can be rebuilt by now, or a
    /// Keychain that cannot be read now, must cost no pairing.
    @Test func resetDeletesNothingUnlessTheRebuildStillFails() throws {
        let item = item()
        defer { remove(item.service, item.account) }
        try add(item.service, item.account, in: own, new)
        try add(item.service, item.account, in: shared, old)

        EnclaveBlob.reset([item]) {}
        EnclaveBlob.reset([item]) { throw KeychainError(status: errSecInteractionNotAllowed) }

        #expect(copies(item.service, item.account) == [own: new, shared: old])
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
