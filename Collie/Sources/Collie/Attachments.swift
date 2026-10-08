import CollieCore
import Foundation
import ImageIO
import UIKit
import UniformTypeIdentifiers

struct PendingAttachment: Sendable {
    let name: String
    /// Gets the size limit; runs off the main actor.
    let load: @Sendable (UInt64) async throws -> Data
}

struct AttachedFile: Identifiable {
    enum Kind { case image, file }

    let id = UUID()
    let path: String
    let name: String
    let kind: Kind
    var thumbnail: UIImage?
    var uploaded = Date.now

    init(path: String, name: String) {
        self.path = path
        self.name = name
        kind = Self.type(of: name)?.conforms(to: .image) == true ? .image : .file
    }

    var symbol: String {
        switch kind {
        case .image: "photo"
        case .file: Self.type(of: name)?.conforms(to: .text) == true ? "doc.text" : "doc"
        }
    }

    private static func type(of name: String) -> UTType? {
        UTType(filenameExtension: (name as NSString).pathExtension)
    }
}

extension AttachedFile: Codable {
    private enum CodingKeys: String, CodingKey {
        case path, name, uploaded, thumbnail
    }

    init(from decoder: any Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        self.init(path: try container.decode(String.self, forKey: .path), name: try container.decode(String.self, forKey: .name))
        uploaded = try container.decode(Date.self, forKey: .uploaded)
        thumbnail = try container.decodeIfPresent(Data.self, forKey: .thumbnail).flatMap(UIImage.init(data:))
    }

    func encode(to encoder: any Encoder) throws {
        var container = encoder.container(keyedBy: CodingKeys.self)
        try container.encode(path, forKey: .path)
        try container.encode(name, forKey: .name)
        try container.encode(uploaded, forKey: .uploaded)
        try container.encodeIfPresent(thumbnail?.jpegData(compressionQuality: 0.8), forKey: .thumbnail)
    }
}

struct AttachmentUpload: Equatable {
    let id: UUID
    let name: String
    var index = 1
    var count = 1
    var sent: UInt64 = 0
    var total: UInt64 = 0

    var fraction: Double? { total == 0 ? nil : Double(sent) / Double(total) }
}

enum AttachmentError: LocalizedError, Equatable {
    case tooLarge(limit: UInt64)
    case tooMany(limit: Int)
    case empty
    case unreadablePhoto

    var errorDescription: String? {
        switch self {
        case .tooLarge(let limit):
            "Attachments are limited to \(ByteCountFormatter.string(fromByteCount: Int64(limit), countStyle: .memory))."
        case .tooMany(let limit): "Up to \(limit) attachments per prompt."
        case .empty: "The file is empty."
        case .unreadablePhoto: "The photo could not be read."
        }
    }
}

enum Attachment {
    /// Mirrors the protocol's MAX_ATTACHMENT_NAME_CHARS; the Mac sanitizes the name again.
    static let maxNameChars = 64
    static let maxPerPrompt = 10
    static let photoMaxSide: CGFloat = 2048
    static let photoQuality: CGFloat = 0.85
    static let thumbnailMaxSide: CGFloat = 192
    /// collied deletes an upload 24 h after it arrives; the hour less allows for clock skew.
    static let keptOnMachine: TimeInterval = 23 * 60 * 60

    static func photoName(at date: Date, index: Int = 1, timeZone: TimeZone = .current) -> String {
        let formatter = DateFormatter()
        formatter.locale = Locale(identifier: "en_US_POSIX")
        formatter.calendar = Calendar(identifier: .gregorian)
        formatter.timeZone = timeZone
        formatter.dateFormat = "yyyyMMdd-HHmmss"
        let suffix = index > 1 ? "-\(index)" : ""
        return "photo-\(formatter.string(from: date))\(suffix).jpg"
    }

    /// Counts Unicode scalars like Rust's `chars()`, keeping the extension when it has to cut.
    static func suggestedName(_ fileName: String) -> String {
        // AttachmentName refuses these; collied renames the file anyway, so dropping them is harmless.
        let kept = String(String.UnicodeScalarView(fileName.unicodeScalars.filter {
            !["/", "\\"].contains($0) && ![.control, .format].contains($0.properties.generalCategory)
        }))
        let name = kept.isEmpty || kept == "." || kept == ".." ? "attachment" : kept
        guard name.unicodeScalars.count > maxNameChars else { return name }
        let ext = (name as NSString).pathExtension
        let suffix = ext.isEmpty || ext.unicodeScalars.count > 16 ? "" : "." + ext
        let stem = suffix.isEmpty ? name : String(name.dropLast(suffix.count))
        var scalars = String.UnicodeScalarView(stem.unicodeScalars.prefix(maxNameChars - suffix.unicodeScalars.count))
        while scalars.last.map({ $0.properties.isWhitespace || $0 == "." }) == true { scalars.removeLast() }
        return String(scalars) + suffix
    }

    static func check(_ data: Data, limit: UInt64) throws {
        guard !data.isEmpty else { throw AttachmentError.empty }
        guard UInt64(data.count) <= limit else { throw AttachmentError.tooLarge(limit: limit) }
    }

    /// Claude Code does not read HEIC, so photos always go out as JPEG; drawing also applies the EXIF orientation.
    static func jpeg(from data: Data) throws -> Data {
        guard let image = UIImage(data: data), image.size.width > 0, image.size.height > 0 else {
            throw AttachmentError.unreadablePhoto
        }
        let scale = min(1, photoMaxSide / max(image.size.width, image.size.height))
        let size = CGSize(width: (image.size.width * scale).rounded(), height: (image.size.height * scale).rounded())
        let format = UIGraphicsImageRendererFormat()
        format.scale = 1
        format.opaque = true
        let jpeg = UIGraphicsImageRenderer(size: size, format: format).jpegData(withCompressionQuality: photoQuality) { _ in
            image.draw(in: CGRect(origin: .zero, size: size))
        }
        guard !jpeg.isEmpty else { throw AttachmentError.unreadablePhoto }
        return jpeg
    }

    /// ImageIO decodes straight to the thumbnail size, so a huge image never needs a full-size buffer.
    static func thumbnail(from data: Data) async -> UIImage? {
        guard let source = CGImageSourceCreateWithData(data as CFData, nil) else { return nil }
        let options: [CFString: Any] = [
            kCGImageSourceCreateThumbnailFromImageAlways: true,
            kCGImageSourceCreateThumbnailWithTransform: true,
            kCGImageSourceThumbnailMaxPixelSize: thumbnailMaxSide,
        ]
        return CGImageSourceCreateThumbnailAtIndex(source, 0, options as CFDictionary).map { UIImage(cgImage: $0) }
    }

    /// The size is checked before reading so a huge file is never loaded.
    static func read(_ url: URL, limit: UInt64) throws -> Data {
        let scoped = url.startAccessingSecurityScopedResource()
        defer { if scoped { url.stopAccessingSecurityScopedResource() } }
        if let size = try url.resourceValues(forKeys: [.fileSizeKey]).fileSize, UInt64(size) > limit {
            throw AttachmentError.tooLarge(limit: limit)
        }
        let data = try Data(contentsOf: url)
        try check(data, limit: limit)
        return data
    }
}

final class UploadProgressRelay: UploadProgress {
    private let report: @Sendable (UInt64, UInt64) -> Void

    init(_ report: @escaping @Sendable (UInt64, UInt64) -> Void) {
        self.report = report
    }

    func onProgress(sent: UInt64, total: UInt64) {
        report(sent, total)
    }
}
