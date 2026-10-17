import Foundation
import XCTest

final class SetupCommandTests: XCTestCase {
    func testQuotedExecutableAndSelfCleanup() throws {
        let manager = FileManager.default
        let directory = manager.temporaryDirectory.appending(path: "setup ' space\n$(touch injected)-\(UUID().uuidString)")
        try manager.createDirectory(at: directory, withIntermediateDirectories: false)
        defer { try? manager.removeItem(at: directory) }
        let executable = directory.appending(path: "collied")
        try Data("#!/bin/sh\nprintf '%s' \"$*\" > \"${0%/*}/arguments\"\n".utf8).write(to: executable)
        try manager.setAttributes([.posixPermissions: 0o700], ofItemAtPath: executable.path)
        let command = try SetupCommand.create(executable: executable)
        defer { try? command.remove() }
        for url in [command.file, command.file.deletingLastPathComponent()] {
            let attributes = try manager.attributesOfItem(atPath: url.path)
            XCTAssertEqual((attributes[.posixPermissions] as? NSNumber)?.intValue, 0o700)
        }
        let process = Process()
        process.executableURL = command.file
        try process.run()
        process.waitUntilExit()
        XCTAssertEqual(process.terminationStatus, 0)
        XCTAssertEqual(try String(contentsOf: directory.appending(path: "arguments"), encoding: .utf8), "setup")
        XCTAssertFalse(manager.fileExists(atPath: command.file.deletingLastPathComponent().path))
        XCTAssertFalse(manager.fileExists(atPath: directory.appending(path: "injected").path))
    }

    func testRejectsMissingNonExecutableDirectoryAndSymlink() throws {
        let manager = FileManager.default
        let directory = manager.temporaryDirectory.appending(path: UUID().uuidString)
        try manager.createDirectory(at: directory, withIntermediateDirectories: false)
        defer { try? manager.removeItem(at: directory) }
        let executable = directory.appending(path: "collied")
        XCTAssertThrowsError(try SetupCommand.create(executable: executable))
        XCTAssertThrowsError(try SetupCommand.create(executable: directory))
        try Data().write(to: executable)
        try manager.setAttributes([.posixPermissions: 0o600], ofItemAtPath: executable.path)
        XCTAssertThrowsError(try SetupCommand.create(executable: executable))
        try manager.setAttributes([.posixPermissions: 0o700], ofItemAtPath: executable.path)
        let link = directory.appending(path: "link")
        try manager.createSymbolicLink(at: link, withDestinationURL: executable)
        XCTAssertThrowsError(try SetupCommand.create(executable: link))
    }

    func testFailedOpenCleanupIsExactAndRepeatable() throws {
        let command = try SetupCommand.create(executable: URL(filePath: "/usr/bin/true"))
        let directory = command.file.deletingLastPathComponent()
        let unrelated = directory.appending(path: "keep")
        try Data("keep".utf8).write(to: unrelated)
        XCTAssertThrowsError(try command.remove())
        XCTAssertFalse(FileManager.default.fileExists(atPath: command.file.path))
        XCTAssertEqual(try String(contentsOf: unrelated, encoding: .utf8), "keep")
        try FileManager.default.removeItem(at: unrelated)
        try command.remove()
        try command.remove()
    }
}
