import CollieCore
import Foundation
import PDFKit
import SwiftUI
import Testing
import UIKit
import zlib

@testable import Collie

private func found(_ text: String) -> [String] {
    FileLinks.paths(in: text).map(\.path)
}

@Test func chatRecognizesAbsoluteRelativeAndNamedFiles() {
    #expect(found("Saved /Users/me/src/app/shot.png and src/main.rs:42, see README.md.") == ["/Users/me/src/app/shot.png", "src/main.rs", "README.md"])
    #expect(found(#"{"file_path":"/Users/me/app/out.pdf"}"#) == ["/Users/me/app/out.pdf"])
    #expect(found("(./build.sh) `docs/a b.md`") == ["./build.sh", "b.md"])
    #expect(found("Run /compact, e.g. and/or Node.js v1.2.3 km/h https://example.com/a.png ../x.rs .env src/*.rs a=b/c.md") == [])
}

@Test func terminalSelectionsOpenOnlyOnePath() {
    #expect(FileLinks.path(selected: "/Users/me/git/chewtoy/test.txt") == "/Users/me/git/chewtoy/test.txt")
    #expect(FileLinks.path(selected: "I created test.txt with") == "test.txt")
    #expect(FileLinks.path(selected: "src/a.rs and src/b.rs") == nil)
    #expect(FileLinks.path(selected: "with one line") == nil)
}

@Test func linksRoundTripToThePathAndSkipExistingLinks() throws {
    var text = try AttributedString(markdown: "Open [the shot](shots/a%20b.png#L3) or src/main.rs:7 or [site](https://example.com).")
    text = FileLinks.linked(text)
    let links = text.runs.compactMap(\.link)
    #expect(links.compactMap(FileLinks.path(from:)) == ["shots/a b.png", "src/main.rs"])
    #expect(links.contains { $0.scheme == "https" })
    #expect(FileLinks.path(from: try #require(URL(string: "file:///Users/me/x.txt"))) == "/Users/me/x.txt")
    #expect(FileLinks.path(from: try #require(URL(string: "mailto:a@example.com"))) == nil)
    #expect(FileLinks.path(from: try #require(URL(string: "#top"))) == nil)
}

@MainActor @Test func tappingAFileLinkOpensTheViewer() throws {
    let agent = agentModel(FakeCore())
    let url = try #require(FileLinks.linked(AttributedString("see src/main.rs")).runs.compactMap(\.link).first)
    _ = ConversationMarkdown.open(url, agent: agent)
    let link = try #require(agent.fileLink)
    #expect(link.path == "src/main.rs")
    _ = ConversationMarkdown.open(try #require(URL(string: "https://example.com")), agent: agent)
    #expect(agent.fileLink == link)
}

@MainActor @Test func viewerKeepsATemporaryCopyUntilItCloses() async throws {
    let model = FileViewerModel(path: "src/main.rs")
    var asked: [String] = []
    await model.load { path in
        asked.append(path)
        return OpenedFile(name: "main.rs", size: 22, kind: .text, data: Data("fn main() {}\n// done\n".utf8))
    }
    #expect(asked == ["src/main.rs"])
    #expect(model.lines == ["fn main() {}", "// done", ""])
    let url = try #require(model.shareURL)
    #expect(url.lastPathComponent == "main.rs")
    #expect(try Data(contentsOf: url) == Data("fn main() {}\n// done\n".utf8))
    model.close()
    #expect(model.shareURL == nil)
    #expect(!FileManager.default.fileExists(atPath: url.deletingLastPathComponent().path))
}

@MainActor @Test func launchRemovesACopyLeftByAKilledViewer() async throws {
    let model = FileViewerModel(path: "notes.txt")
    await model.load { _ in OpenedFile(name: "notes.txt", size: 2, kind: .text, data: Data("hi".utf8)) }
    let url = try #require(model.shareURL)
    #expect(FileManager.default.fileExists(atPath: url.path))
    FileViewerModel.purgeCopies()
    #expect(!FileManager.default.fileExists(atPath: url.path))
}

@MainActor @Test func viewerShowsImagesAndRefusals() async throws {
    let png = try #require(UIGraphicsImageRenderer(size: CGSize(width: 2, height: 2)).image { _ in }.pngData())
    let model = FileViewerModel(path: "shot.png")
    await model.load { _ in OpenedFile(name: "shot.png", size: UInt64(png.count), kind: .image, data: png) }
    #expect(model.image != nil)
    model.close()

    let old = FileViewerModel(path: "a.pdf")
    await old.load { _ in throw CoreError.NotImplemented }
    #expect(old.error == "Update collied to open files.")
    #expect(old.file == nil)

    let big = FileViewerModel(path: "big.log")
    await big.load { _ in throw CoreError.TooLarge(message: "big.log is 3.2 MB; text files open up to 1 MB.") }
    #expect(big.error == "big.log is 3.2 MB; text files open up to 1 MB.")
}

@MainActor @Test func viewerDecodesAnImageNoLargerThanTheScreen() async throws {
    let side = try #require(FileViewerModel.screenSide)
    let format = UIGraphicsImageRendererFormat()
    format.scale = 1
    let png = try #require(UIGraphicsImageRenderer(size: CGSize(width: side * 2, height: 4), format: format).image { _ in }.pngData())
    let model = FileViewerModel(path: "wide.png")
    await model.load { _ in OpenedFile(name: "wide.png", size: UInt64(png.count), kind: .image, data: png) }
    let image = try #require(model.image?.cgImage)
    #expect(image.width == Int(side))
    #expect(image.height == 2)
    model.close()
}

@MainActor @Test func viewerDoesNotDecodeAnImageOverThePixelLimit() async throws {
    func bigEndian(_ value: Int) -> [UInt8] { withUnsafeBytes(of: UInt32(value).bigEndian) { Array($0) } }
    func chunk(_ kind: String, _ body: [UInt8]) -> [UInt8] {
        let checked = Array(kind.utf8) + body
        return bigEndian(body.count) + checked + bigEndian(Int(crc32(0, checked, uInt(checked.count))))
    }
    // A one-bit PNG just over the limit, all zeros: each row is a filter byte and 1563 bytes of pixels.
    let (width, height) = (12500, 8001)
    let rows = [UInt8](repeating: 0, count: height * (1 + (width + 7) / 8))
    var packed = [UInt8](repeating: 0, count: Int(compressBound(uLong(rows.count))))
    var length = uLong(packed.count)
    #expect(compress(&packed, &length, rows, uLong(rows.count)) == Z_OK)
    let png = Data(
        [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A] + chunk("IHDR", bigEndian(width) + bigEndian(height) + [1, 0, 0, 0, 0])
            + chunk("IDAT", Array(packed[..<Int(length)])) + chunk("IEND", [])
    )
    #expect(FileViewerModel.pixels(in: png) == width * height)
    let model = FileViewerModel(path: "huge.png")
    await model.load { _ in OpenedFile(name: "huge.png", size: UInt64(png.count), kind: .image, data: png) }
    #expect(model.oversized)
    #expect(model.image == nil)
    #expect(model.shareURL != nil)
    model.close()
}

@MainActor @Test func pdfPreviewTakesLinkClicksItself() async throws {
    let host = UIHostingController(rootView: PDFPreview(document: PDFDocument()))
    try await hosting(host) { _ in
        let view = try await waitFor("the PDF view") { descendants(host.view).compactMap { $0 as? PDFView }.first }
        #expect(view.delegate?.responds(to: #selector(PDFViewDelegate.pdfViewWillClick(onLink:with:))) == true)
    }
}
