#if DEBUG && targetEnvironment(simulator)
import CollieCore
import Darwin
import Foundation
import Security

enum SimulatorVM {
    static var enabled: Bool { ProcessInfo.processInfo.environment["COLLIE_VM_TEST"] == "1" }

    static func verifyCore() throws {
        guard !buildInfo().release,
              let symbol = dlsym(UnsafeMutableRawPointer(bitPattern: -2), "collie_vm_test_supported"),
              unsafeBitCast(symbol, to: (@convention(c) () -> UInt32).self)() == 0x434f4c4c
        else {
            try recordIdentityMetadata(file: "vm-core-rejection.json")
            throw CocoaError(.featureUnsupported)
        }
    }

    private static func recordIdentityMetadata(file: String) throws {
        let services = ["dev.rbstp.collie.identity", "dev.rbstp.collie.vm-test.dev.rbstp.collie.identity"]
        var counts: [[String: Any]] = []
        for service in services {
            for account in ["tls", "terminal"] {
                let query: [String: Any] = [
                    kSecClass as String: kSecClassGenericPassword,
                    kSecAttrService as String: service,
                    kSecAttrAccount as String: account,
                    kSecUseDataProtectionKeychain as String: true,
                    kSecMatchLimit as String: kSecMatchLimitAll,
                    kSecReturnAttributes as String: true,
                ]
                var attributes: CFTypeRef?
                let status = SecItemCopyMatching(query as CFDictionary, &attributes)
                counts.append(["service": service, "account": account, "status": status,
                               "count": (attributes as? [[String: Any]])?.count ?? 0])
            }
        }
        let path = URL.temporaryDirectory.appending(path: file)
        let descriptor = open(path.path, O_WRONLY | O_CREAT | O_EXCL | O_NOFOLLOW, 0o600)
        if descriptor < 0, errno == EEXIST { return }
        guard descriptor >= 0 else { throw CocoaError(.fileWriteNoPermission) }
        defer { close(descriptor) }
        try FileHandle(fileDescriptor: descriptor, closeOnDealloc: false).write(contentsOf: JSONSerialization.data(withJSONObject: counts))
    }

    static func pair(core: CollieCore) async throws -> Machine {
        let file = try StateDirectory.prepare().appending(path: "vm-pairing.txt")
        let descriptor = open(file.path, O_RDONLY | O_NOFOLLOW)
        guard descriptor >= 0 else { throw CocoaError(.fileReadNoPermission) }
        defer { close(descriptor) }
        var metadata = stat()
        guard fstat(descriptor, &metadata) == 0, metadata.st_uid == geteuid(),
              metadata.st_mode & 0o170000 == 0o100000, metadata.st_mode & 0o077 == 0,
              metadata.st_size <= 8192,
              let data = try FileHandle(fileDescriptor: descriptor, closeOnDealloc: false).read(upToCount: 8193),
              data.count <= 8192, let text = String(data: data, encoding: .utf8)
        else { throw CocoaError(.fileReadNoPermission) }
        let uri = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard uri.hasPrefix("collie://pair#") else { throw CocoaError(.fileReadCorruptFile) }
        let machine = try await core.pair(inviteUri: uri, deviceLabel: "VM Simulator")
        try recordIdentityMetadata(file: "vm-core-identity.json")
        try FileManager.default.removeItem(at: file)
        return machine
    }
}
#endif
