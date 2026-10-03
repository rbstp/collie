import CollieCore
import Foundation
import Testing
import UIKit

@testable import Collie

@MainActor
private func agentModel(_ core: FakeCore) -> AgentModel {
    AgentModel(core: core, route: AgentRoute(machineId: "m1", terminalId: "term_1"), prefsFile: nil)
}

private let path = "/Users/me/Library/Caches/dev.rbstp.collied/attachments/0123456789abcdef/notes.txt"

@Test(arguments: [
    ("", "\(path) "),
    ("look at", "look at \(path) "),
    ("look at ", "look at \(path) "),
    ("look at\n", "look at\n\(path) "),
])
func pathIsAppendedToTheDraftWithSpaces(draft: String, expected: String) {
    #expect(Attachment.appending(path, to: draft) == expected)
}

@Test func photoNameUsesTheLocalTimestamp() throws {
    let utc = try #require(TimeZone(identifier: "UTC"))
    let montreal = try #require(TimeZone(identifier: "America/Montreal"))
    let date = Date(timeIntervalSince1970: 1_791_000_045)
    #expect(Attachment.photoName(at: date, timeZone: utc) == "photo-20261003-040045.jpg")
    #expect(Attachment.photoName(at: date, timeZone: montreal) == "photo-20261003-000045.jpg")
}

@Test func longNamesAreCutToTheProtocolLimitKeepingTheExtension() {
    #expect(Attachment.suggestedName("report.pdf") == "report.pdf")
    #expect(Attachment.suggestedName("") == "attachment")
    #expect(Attachment.suggestedName("..") == "attachment")
    #expect(Attachment.suggestedName("a\u{200D}b\\c\u{0007}.png") == "abc.png")
    #expect(Attachment.suggestedName("\u{202E}") == "attachment")
    let long = String(repeating: "a", count: 100) + ".pdf"
    let cut = Attachment.suggestedName(long)
    #expect(cut.unicodeScalars.count == Attachment.maxNameChars)
    #expect(cut.hasSuffix("aaa.pdf"))
    let accented = String(repeating: "é", count: 80) + ".png"
    #expect(Attachment.suggestedName(accented).unicodeScalars.count <= Attachment.maxNameChars)
}

@Test func photosAreExportedAsJpegCappedAt2048() throws {
    let format = UIGraphicsImageRendererFormat()
    format.scale = 1
    let png = UIGraphicsImageRenderer(size: CGSize(width: 4000, height: 1000), format: format).pngData { context in
        UIColor.systemTeal.setFill()
        context.fill(CGRect(x: 0, y: 0, width: 4000, height: 1000))
    }
    let jpeg = try Attachment.jpeg(from: png)
    #expect(jpeg.prefix(2) == Data([0xFF, 0xD8]))
    let image = try #require(UIImage(data: jpeg))
    #expect(image.size == CGSize(width: 2048, height: 512))

    let small = UIGraphicsImageRenderer(size: CGSize(width: 300, height: 200), format: format).pngData { _ in }
    #expect(try UIImage(data: Attachment.jpeg(from: small))?.size == CGSize(width: 300, height: 200))
    #expect(throws: AttachmentError.unreadablePhoto) { try Attachment.jpeg(from: Data("not an image".utf8)) }
}

@MainActor
@Test func uploadShowsProgressThenAppendsThePath() async {
    let core = FakeCore()
    let model = agentModel(core)
    model.draft = "summarize"
    core.set(hold: true)
    let upload = model.attach(name: "notes.txt") { _ in Data(repeating: 7, count: 100_000) }
    #expect(upload != nil)
    #expect(model.upload?.name == "notes.txt")
    #expect(model.attach(name: "other.txt") { _ in Data([1]) } == nil)
    await core.waitHeld(1)
    while model.upload?.sent != 50_000 { await Task.yield() }
    #expect(model.upload?.fraction == 0.5)
    core.release()
    await upload?.value
    #expect(model.upload == nil)
    #expect(model.promptError == nil)
    #expect(model.draft == "summarize \(path) ")
    #expect(core.snapshot.uploads == ["notes.txt 100000"])
}

@MainActor
@Test func cancelledUploadLeavesTheDraftAlone() async {
    let core = FakeCore()
    let model = agentModel(core)
    model.draft = "keep"
    core.set(hold: true)
    let upload = model.attach(name: "notes.txt") { _ in Data([1, 2, 3]) }
    await core.waitHeld(1)
    model.cancelUpload()
    #expect(model.upload == nil)
    #expect(core.snapshot.cancelledUploads == ["m1"])
    model.cancelUpload()
    #expect(core.snapshot.cancelledUploads == ["m1"])
    core.release()
    await upload?.value
    #expect(model.draft == "keep")
    #expect(model.promptError == nil)
    #expect(model.upload == nil)
}

@MainActor
@Test func failedOrOversizedUploadShowsAnError() async {
    let core = FakeCore()
    let model = agentModel(core)
    core.set(error: .MachineNotFound)
    await model.attach(name: "notes.txt") { _ in Data([1]) }?.value
    #expect(model.promptError == CoreError.MachineNotFound.description)
    #expect(model.upload == nil)
    #expect(model.draft.isEmpty)

    core.set()
    core.state.withLock { $0.maxAttachmentBytes = 4 }
    await model.attach(name: "big.bin") { _ in Data(count: 5) }?.value
    #expect(model.promptError == AttachmentError.tooLarge(limit: 4).errorDescription)
    await model.attach(name: "empty.txt") { _ in Data() }?.value
    #expect(model.promptError == AttachmentError.empty.errorDescription)
    #expect(core.snapshot.uploads == ["notes.txt 1"])
}

@MainActor
@Test func batchesUploadInOrderUpToTenPerPrompt() async {
    let core = FakeCore()
    let model = agentModel(core)
    let items = (1...12).map { i in PendingAttachment(name: "f\(i).txt") { _ in i == 2 ? Data() : Data([1]) } }
    await model.attach(items)?.value
    #expect(core.snapshot.uploads == ["f1.txt 1"] + (3...10).map { "f\($0).txt 1" })
    #expect(model.promptError == AttachmentError.tooMany(limit: 10).errorDescription)
    #expect(model.attachmentSlots == 1)
    #expect(model.upload == nil)
    await model.attach(items.suffix(2))?.value
    #expect(model.attachmentSlots == 0)
    #expect(model.attach(items) == nil)
    model.draft = ""
    #expect(model.attachmentSlots == 10)
}

@Test func filesOverTheLimitAreRefusedBeforeReading() throws {
    let dir = FileManager.default.temporaryDirectory.appending(path: UUID().uuidString, directoryHint: .isDirectory)
    try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: dir) }
    let file = dir.appending(path: "notes.txt")
    try Data(repeating: 1, count: 10).write(to: file)
    #expect(try Attachment.read(file, limit: 10) == Data(repeating: 1, count: 10))
    #expect(throws: AttachmentError.tooLarge(limit: 9)) { try Attachment.read(file, limit: 9) }
}
