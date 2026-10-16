import Darwin
import Foundation

struct SetupCommand {
    let file: URL

    static func create(executable: URL) throws -> SetupCommand {
        let manager = FileManager.default
        let attributes = try? manager.attributesOfItem(atPath: executable.path)
        guard attributes?[.type] as? FileAttributeType == .typeRegular,
            manager.isExecutableFile(atPath: executable.path)
        else {
            throw NSError(domain: "CollieSetup", code: 1, userInfo: [
                NSLocalizedDescriptionKey: "Install collied first, then choose Set Up again."
            ])
        }
        let directory = manager.temporaryDirectory.appending(path: "collie-setup-\(UUID().uuidString)")
        try manager.createDirectory(at: directory, withIntermediateDirectories: false, attributes: [.posixPermissions: 0o700])
        let command = SetupCommand(file: directory.appending(path: "Setup.command"))
        do {
            let quoted = "'" + executable.path.replacingOccurrences(of: "'", with: "'\\''") + "'"
            let script = """
                #!/bin/sh
                /bin/rm -- "$0" || exit 1
                /bin/rmdir -- "${0%/*}" || exit 1
                exec \(quoted) setup
                """ + "\n"
            try Data(script.utf8).write(to: command.file, options: .withoutOverwriting)
            try manager.setAttributes([.posixPermissions: 0o700], ofItemAtPath: command.file.path)
            return command
        } catch {
            try? command.remove()
            throw error
        }
    }

    func remove() throws {
        if unlink(file.path) != 0 && errno != ENOENT {
            throw POSIXError(POSIXErrorCode(rawValue: errno) ?? .EIO)
        }
        if rmdir(file.deletingLastPathComponent().path) != 0 && errno != ENOENT {
            throw POSIXError(POSIXErrorCode(rawValue: errno) ?? .EIO)
        }
    }
}
