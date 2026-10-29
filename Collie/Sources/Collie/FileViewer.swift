import CollieCore
import PDFKit
import SwiftUI
import UIKit
import UniformTypeIdentifiers

struct FileLink: Identifiable, Equatable {
    let id = UUID()
    let path: String
}

/// File paths in Chat: absolute ones, relative ones with a folder (`src/main.rs`) and bare
/// names with a common file extension (`README.md`). Collied decides what may be read.
enum FileLinks {
    static let scheme = "collie-file"
    private static let delimiters = CharacterSet.whitespacesAndNewlines.union(CharacterSet(charactersIn: "\"'`()[]{}<>,;|"))
    private static let bareExtensions: Set<String> = [
        "md", "txt", "log", "json", "yaml", "yml", "toml", "csv", "pdf", "png", "jpg", "jpeg", "gif", "heic", "webp",
        "rs", "swift", "py", "ts", "tsx", "sh", "html", "css", "xml", "sql", "diff", "patch",
    ]

    static func paths(in text: String) -> [(range: Range<String.Index>, path: String)] {
        var result: [(Range<String.Index>, String)] = []
        var start = text.startIndex
        for index in text.indices {
            if text[index].unicodeScalars.allSatisfy(delimiters.contains) {
                if let found = path(in: text[start..<index]) { result.append(found) }
                start = text.index(after: index)
            }
        }
        if let found = path(in: text[start...]) { result.append(found) }
        return result
    }

    static func linked(_ text: AttributedString) -> AttributedString {
        var result = text
        for (range, path) in paths(in: String(text.characters)) {
            var components = URLComponents()
            components.scheme = scheme
            components.path = path
            guard let url = components.url,
                  let lower = AttributedString.Index(range.lowerBound, within: result),
                  let upper = AttributedString.Index(range.upperBound, within: result),
                  result[lower..<upper].runs.allSatisfy({ $0.link == nil }) else { continue }
            result[lower..<upper].link = url
        }
        return result
    }

    /// A link Chat opens as a file: one it made, or a Markdown link with no scheme or `file:`.
    static func path(from url: URL) -> String? {
        guard let components = URLComponents(url: url, resolvingAgainstBaseURL: false) else { return nil }
        let path: Substring
        switch url.scheme?.lowercased() {
        case scheme: path = Substring(components.path)
        case nil, "file": path = withoutLine(Substring(components.path))
        default: return nil
        }
        return path.isEmpty ? nil : String(path)
    }

    private static func path(in token: Substring) -> (Range<String.Index>, String)? {
        var token = token
        while let last = token.last, ".:!?".contains(last) { token = token.dropLast() }
        let path = withoutLine(token)
        return isPath(path) ? (token.startIndex..<token.endIndex, String(path)) : nil
    }

    private static func withoutLine(_ text: Substring) -> Substring {
        var text = text
        for _ in 0..<2 {
            guard let colon = text.lastIndex(of: ":") else { break }
            let digits = text[text.index(after: colon)...]
            guard !digits.isEmpty, digits.allSatisfy({ $0.isASCII && $0.isNumber }) else { break }
            text = text[..<colon]
        }
        return text
    }

    private static func isPath(_ text: Substring) -> Bool {
        guard !text.isEmpty, !text.contains("://"), !text.hasPrefix("//"), !text.contains(where: { "*$=@\\".contains($0) }) else { return false }
        let parts = text.split(separator: "/", omittingEmptySubsequences: false)
        guard !parts.contains("..") else { return false }
        if text.hasPrefix("/") { return parts.filter { !$0.isEmpty }.count >= 2 }
        let name = parts.last ?? ""
        if text.hasPrefix("./") { return !name.isEmpty }
        guard let dot = name.lastIndex(of: "."), dot != name.startIndex else { return false }
        let ext = name[name.index(after: dot)...]
        guard (1...10).contains(ext.count), ext.allSatisfy({ $0.isASCII && ($0.isLetter || $0.isNumber) }) else { return false }
        if parts.count > 1 { return parts.allSatisfy { !$0.isEmpty } && ext.contains(where: \.isLetter) }
        return bareExtensions.contains(ext.lowercased())
    }
}

@MainActor
@Observable
final class FileViewerModel {
    typealias Open = @MainActor (String) async throws -> OpenedFile

    let path: String
    private(set) var file: OpenedFile?
    private(set) var lines: [String] = []
    private(set) var image: UIImage?
    private(set) var pdf: PDFDocument?
    private(set) var shareURL: URL?
    private(set) var error: String?
    private(set) var loading = false
    private var folder: URL?

    init(path: String) {
        self.path = path
    }

    var name: String { file?.name ?? URL(fileURLWithPath: path).lastPathComponent }

    func load(_ open: Open) async {
        guard !loading else { return }
        loading = true
        error = nil
        defer { loading = false }
        do {
            let file = try await open(path)
            guard !Task.isCancelled else { return }
            close()
            lines = file.kind == .text ? String(decoding: file.data, as: UTF8.self).components(separatedBy: "\n") : []
            image = file.kind == .image ? UIImage(data: file.data) : nil
            pdf = file.kind == .pdf ? PDFDocument(data: file.data) : nil
            let folder = FileManager.default.temporaryDirectory.appending(path: UUID().uuidString, directoryHint: .isDirectory)
            self.folder = folder
            try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
            let url = folder.appending(path: file.name)
            try file.data.write(to: url, options: .completeFileProtection)
            shareURL = url
            self.file = file
        } catch CoreError.Cancelled {
        } catch {
            self.error = Self.message(for: error)
        }
    }

    /// Removes the temporary copy kept for Share.
    func close() {
        if let folder { try? FileManager.default.removeItem(at: folder) }
        folder = nil
        shareURL = nil
    }

    func copy() {
        guard let file else { return }
        switch file.kind {
        case .text:
            UIPasteboard.general.string = String(decoding: file.data, as: UTF8.self)
        case .image where image != nil:
            UIPasteboard.general.image = image
        default:
            let type = UTType(filenameExtension: URL(fileURLWithPath: file.name).pathExtension) ?? .data
            UIPasteboard.general.setData(file.data, forPasteboardType: type.identifier)
        }
    }

    static func message(for error: any Error) -> String {
        if case .NotImplemented = error as? CoreError { return "Update collied to open files." }
        return AgentModel.message(for: error)
    }
}

struct FileViewer: View {
    let agent: AgentModel
    @State private var model: FileViewerModel
    @AppStorage("gitWrapLines") private var wraps = true
    @Environment(\.dismiss) private var dismiss
    @Environment(\.neutralAccent) private var neutralAccent

    init(agent: AgentModel, link: FileLink) {
        self.agent = agent
        _model = State(initialValue: FileViewerModel(path: link.path))
    }

    var body: some View {
        NavigationStack {
            content
                .navigationTitle(model.name)
                .navigationBarTitleDisplayMode(.inline)
                .toolbar {
                    ToolbarItem(placement: .cancellationAction) {
                        Button("Done", systemImage: "xmark") { dismiss() }
                    }
                    if model.file?.kind == .text {
                        ToolbarItem(placement: .primaryAction) {
                            Button { wraps.toggle() } label: { Label(wraps ? "Wrap lines on" : "Wrap lines off", image: "WrapLines") }
                                .tint(wraps ? Color.accentColor : neutralAccent ? .secondary : .primary)
                        }
                    }
                    if model.file != nil {
                        ToolbarItem(placement: .primaryAction) {
                            Button("Copy", systemImage: "doc.on.doc") { model.copy() }
                        }
                    }
                    if let url = model.shareURL {
                        ToolbarItem(placement: .primaryAction) {
                            ShareLink(item: url)
                        }
                    }
                }
        }
        .task { await model.load(agent.openFile) }
        .onDisappear {
            if model.loading { agent.cancelFileOpen() }
            model.close()
        }
    }

    @ViewBuilder private var content: some View {
        if let file = model.file {
            switch file.kind {
            case .text:
                ScrollView(wraps ? .vertical : [.vertical, .horizontal]) {
                    LazyVStack(alignment: .leading, spacing: 0) {
                        ForEach(model.lines.indices, id: \.self) { index in
                            Text(verbatim: model.lines[index].isEmpty ? " " : model.lines[index])
                                .fixedSize(horizontal: !wraps, vertical: true)
                                .frame(maxWidth: wraps ? .infinity : nil, alignment: .leading)
                        }
                    }
                    .font(.system(.caption, design: .monospaced))
                    .textSelection(.enabled)
                    .padding()
                }
            case .image where model.image != nil:
                ScrollView {
                    Image(uiImage: model.image ?? UIImage()).resizable().scaledToFit().padding()
                        .accessibilityLabel(file.name)
                }
            case .pdf where model.pdf != nil:
                PDFPreview(document: model.pdf).ignoresSafeArea(edges: .bottom)
            default:
                ContentUnavailableView {
                    Label(file.name, systemImage: "doc")
                } description: {
                    Text("\(ByteCountFormatter.string(fromByteCount: Int64(file.size), countStyle: .file)) · No preview")
                }
            }
        } else if let error = model.error {
            ContentUnavailableView {
                Label("Cannot open \(model.name)", systemImage: "exclamationmark.triangle")
            } description: {
                Text(error)
            } actions: {
                Button("Try Again") { Task { await model.load(agent.openFile) } }.buttonStyle(.glass)
            }
        } else {
            ProgressView("Opening \(model.name)…").frame(maxWidth: .infinity, maxHeight: .infinity)
        }
    }
}

private struct PDFPreview: UIViewRepresentable {
    let document: PDFDocument?

    func makeUIView(context: Context) -> PDFView {
        let view = PDFView()
        view.autoScales = true
        return view
    }

    func updateUIView(_ view: PDFView, context: Context) {
        if view.document !== document { view.document = document }
    }
}
