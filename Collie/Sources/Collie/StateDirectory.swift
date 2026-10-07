import Foundation

enum StateDirectory {
    /// collie-core refuses a state dir that is not 0700. The protection class lets a
    /// background launch after first unlock read tailnet state and machines.
    static func prepare() throws -> URL {
        let dir = URL.applicationSupportDirectory.appending(path: "collie", directoryHint: .isDirectory)
        let attributes: [FileAttributeKey: Any] = [
            .posixPermissions: 0o700,
            .protectionKey: FileProtectionType.completeUntilFirstUserAuthentication,
        ]
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true, attributes: attributes)
        try FileManager.default.setAttributes(attributes, ofItemAtPath: dir.path)
        // tsnet state holds the node and machine keys; a restored backup would clone this node.
        var excluded = URLResourceValues()
        excluded.isExcludedFromBackup = true
        var target = dir
        try target.setResourceValues(excluded)
        return dir
    }
}

/// A JSON file in the state dir: a missing or unreadable file loads as the default.
protocol StateFile: Codable {
    init()
}

extension StateFile {
    static func load(from file: URL?) -> Self {
        file.flatMap { try? Data(contentsOf: $0) }.flatMap { try? JSONDecoder().decode(Self.self, from: $0) } ?? Self()
    }

    func save(to file: URL?) {
        guard let file, let data = try? JSONEncoder().encode(self) else { return }
        try? data.write(to: file, options: .atomic)
    }
}
