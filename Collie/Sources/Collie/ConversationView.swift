import CollieCore
import SwiftUI

@MainActor
@Observable
final class ConversationModel {
    private(set) var messages: [HistoryMessage] = []
    private(set) var session: String?
    private(set) var available: Bool?
    private(set) var unavailableReason: String?
    private(set) var before: String?
    private(set) var after: String?
    private(set) var loading = false
    private(set) var omitted = false
    private(set) var error: String?
    private(set) var query = ""
    private(set) var more = false
    private var generation = 0

    typealias Load = @MainActor (String?, String?, String?) async throws -> HistoryPage

    func stop() {
        generation += 1
        loading = false
    }

    static func boundedQuery(_ value: String) -> String {
        var query = ""
        var bytes = 0
        for character in value {
            let size = character.utf8.count
            guard bytes + size <= 256 else { break }
            query.append(character)
            bytes += size
        }
        return query
    }

    func begin(query: String) {
        stop()
        if self.query != query {
            self.query = query
            messages = []
            error = nil
            available = nil
            unavailableReason = nil
            before = nil
            after = nil
            session = nil
            omitted = false
        }
    }

    func fetch(older: Bool = false, load: Load) async {
        guard !loading else { return }
        let ticket = generation
        loading = true
        defer { if ticket == generation { loading = false } }
        do {
            let page = try await load(older ? before : nil, older ? nil : after, query.isEmpty ? nil : query)
            guard ticket == generation, !Task.isCancelled else { return }
            apply(page, older: older)
            error = nil
        } catch {
            guard ticket == generation, !Task.isCancelled else { return }
            self.error = AgentModel.message(for: error)
            if case .NotImplemented = error as? CoreError { available = false }
        }
    }

    func apply(_ page: HistoryPage, older: Bool) {
        let replacing = page.reset || session != page.session || available != true
        if replacing {
            messages = []
            before = nil
            after = nil
            omitted = false
        }
        available = page.available
        unavailableReason = page.unavailableReason
        session = page.session
        guard page.available else { return }
        if !page.messages.isEmpty {
            let updates = Dictionary(page.messages.map { ($0.id, $0) }, uniquingKeysWith: { _, latest in latest })
            messages = messages.map { updates[$0.id] ?? $0 }
            var known = Set(messages.map(\.id))
            let added = page.messages.compactMap { known.insert($0.id).inserted ? updates[$0.id] : nil }
            if older && !replacing { messages.insert(contentsOf: added, at: 0) }
            else { messages.append(contentsOf: added) }
        }
        if older || replacing || after == nil { before = page.before }
        if !older || replacing { after = page.after }
        omitted = omitted || page.omitted
        more = page.more
    }
}

struct ConversationView: View {
    @Bindable var model: ConversationModel
    let load: ConversationModel.Load
    let terminal: () -> Void
    @State private var search = ""
    @State private var submitted = ""
    @State private var visibleID: String?
    @State private var atBottom = true
    @State private var matchIndex = 0
    @Environment(\.scenePhase) private var scenePhase

    var body: some View {
        VStack(spacing: 0) {
            HStack {
                Image(systemName: "magnifyingglass").foregroundStyle(.secondary)
                TextField("Search conversation", text: $search)
                    .submitLabel(.search)
                    .onSubmit { submitted = ConversationModel.boundedQuery(search); matchIndex = 0 }
                if !submitted.isEmpty {
                    Button("Clear", systemImage: "xmark.circle.fill") { search = ""; submitted = "" }
                        .labelStyle(.iconOnly)
                }
            }
            .padding(.horizontal)
            .padding(.vertical, 8)
            .glassEffect(.regular.interactive(), in: .capsule)
            .padding(.horizontal)
            .padding(.vertical, 6)
            if let error = model.error {
                Text(error).font(.footnote).foregroundStyle(.orange).padding(.horizontal)
            }
            if model.available == false {
                ContentUnavailableView {
                    Label("History unavailable", systemImage: "text.bubble")
                } description: {
                    Text(model.unavailableReason ?? "This session has no readable Codex or Claude Code transcript yet. Older daemons need an update for Chat.")
                } actions: {
                    Button("Open Terminal", action: terminal)
                }
            } else {
                history
            }
        }
        .onDisappear { model.stop() }
        .task(id: "\(submitted):\(scenePhase == .active)") {
            model.begin(query: submitted)
            guard scenePhase == .active else { return }
            await model.fetch(load: load)
            while !Task.isCancelled {
                if !submitted.isEmpty {
                    if model.before != nil && model.error == nil {
                        await model.fetch(older: true, load: load)
                    } else { return }
                } else {
                    await model.fetch(load: load)
                }
                try? await Task.sleep(for: .milliseconds(model.more || !submitted.isEmpty ? 150 : 1000))
            }
        }
    }

    private var history: some View {
        ScrollViewReader { proxy in
            VStack(spacing: 0) {
                if !submitted.isEmpty && !model.messages.isEmpty {
                    HStack {
                        Text("\(model.messages.count) matches\(model.before != nil ? ", searching…" : "")")
                        Spacer()
                        Button("Previous match", systemImage: "chevron.up") { navigate(-1, proxy: proxy) }
                        Button("Next match", systemImage: "chevron.down") { navigate(1, proxy: proxy) }
                    }
                    .font(.caption)
                    .labelStyle(.iconOnly)
                    .buttonStyle(.glass)
                    .padding(.horizontal)
                }
                ScrollView {
                    LazyVStack(alignment: .leading, spacing: 16) {
                        if model.before != nil && submitted.isEmpty {
                            Button(model.loading ? "Loading…" : "Load older messages") { Task { await model.fetch(older: true, load: load) } }
                                .disabled(model.loading)
                                .frame(maxWidth: .infinity)
                        }
                        if model.omitted {
                            Text("Some transcript records could not be displayed.")
                                .font(.footnote).foregroundStyle(.secondary)
                        }
                        if model.loading && model.messages.isEmpty { ProgressView().frame(maxWidth: .infinity) }
                        if model.available == true && model.messages.isEmpty && !model.loading {
                            Text(model.before != nil ? "Load older messages to continue." : submitted.isEmpty ? "No messages yet." : "No matches.")
                                .foregroundStyle(.secondary)
                        }
                        ForEach(model.messages, id: \.id) { message in
                            ConversationMessage(message: message).id(message.id)
                        }
                        Color.clear.frame(height: 1).id("bottom")
                    }
                    .scrollTargetLayout()
                    .padding()
                }
                .scrollPosition(id: $visibleID, anchor: .top)
                .defaultScrollAnchor(.bottom, for: .initialOffset)
                .onScrollGeometryChange(for: Bool.self) { geo in
                    geo.contentOffset.y + geo.containerSize.height >= geo.contentSize.height - 60
                } action: { _, bottom in atBottom = bottom }
                .onChange(of: model.session) { _, _ in
                    visibleID = nil
                    proxy.scrollTo("bottom", anchor: .bottom)
                }
                .onChange(of: model.messages.last?.id) { _, _ in
                    if atBottom && submitted.isEmpty { proxy.scrollTo("bottom", anchor: .bottom) }
                }
                if !atBottom && submitted.isEmpty {
                    Button("Latest messages", systemImage: "arrow.down") { proxy.scrollTo("bottom", anchor: .bottom) }
                        .font(.footnote).buttonStyle(.glass).padding(6)
                }
            }
        }
    }

    private func navigate(_ direction: Int, proxy: ScrollViewProxy) {
        guard !model.messages.isEmpty else { return }
        matchIndex = (matchIndex + direction + model.messages.count) % model.messages.count
        proxy.scrollTo(model.messages[matchIndex].id, anchor: .top)
    }
}

private struct ConversationMessage: View {
    let message: HistoryMessage

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text(message.role == "user" ? "You" : message.role == "tool" ? "Tool result" : "Assistant")
                .font(.caption.weight(.semibold)).foregroundStyle(.secondary)
            if let tool = message.tool {
                DisclosureGroup {
                    Text(verbatim: message.text).font(.system(.caption, design: .monospaced)).textSelection(.enabled)
                    if let id = message.callId { Text(verbatim: id).font(.caption2).foregroundStyle(.secondary) }
                    truncation
                } label: {
                    Text(verbatim: tool).font(.callout.weight(.medium))
                }
            } else {
                ConversationMarkdown(text: message.text)
                truncation
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(12)
        .background(message.role == "user" ? Color.accentColor.opacity(0.08) : Color.secondary.opacity(0.06), in: RoundedRectangle(cornerRadius: 12))
    }

    @ViewBuilder private var truncation: some View {
        if message.truncated {
            Text("Long content shortened for display. Search includes the full recorded text within the read limit.")
                .font(.caption).foregroundStyle(.secondary)
        }
    }
}

struct ConversationMarkdown: View {
    let text: String

    static func allowsLink(_ url: URL) -> Bool {
        guard let scheme = url.scheme?.lowercased(), ["http", "https"].contains(scheme) else { return false }
        return url.host?.isEmpty == false
    }

    static func blocks(_ text: String) -> [(code: Bool, text: String)] {
        var result: [(Bool, String)] = []
        var code = false
        var lines: [String] = []
        for line in text.components(separatedBy: "\n") {
            if line.trimmingCharacters(in: .whitespaces).hasPrefix("```") {
                if !lines.isEmpty { result.append((code, lines.joined(separator: "\n"))); lines = [] }
                code.toggle()
            } else { lines.append(line) }
        }
        if !lines.isEmpty { result.append((code, lines.joined(separator: "\n"))) }
        return result
    }

    private func markdownLine(_ line: String) -> some View {
        let trimmed = line.trimmingCharacters(in: .whitespaces)
        let hashes = trimmed.prefix { $0 == "#" }.count
        let heading = (1...6).contains(hashes) && trimmed.dropFirst(hashes).hasPrefix(" ")
        let quote = trimmed.hasPrefix("> ")
        let bullet = trimmed.hasPrefix("- ") || trimmed.hasPrefix("* ")
        let body = heading ? String(trimmed.dropFirst(hashes + 1)) : quote ? String(trimmed.dropFirst(2)) : bullet ? "• " + trimmed.dropFirst(2) : line
        let rendered = (try? AttributedString(markdown: body, options: .init(interpretedSyntax: .inlineOnlyPreservingWhitespace))) ?? AttributedString(body)
        return Text(rendered)
            .font(heading ? (hashes <= 2 ? .headline : .subheadline.weight(.semibold)) : .body)
            .foregroundStyle(quote ? .secondary : .primary)
            .padding(.leading, quote ? 10 : 0)
            .textSelection(.enabled)
    }

    var body: some View {
        let blocks = Self.blocks(text)
        VStack(alignment: .leading, spacing: 8) {
            ForEach(blocks.indices, id: \.self) { index in
                let block = blocks[index]
                if block.code {
                    ScrollView(.horizontal) {
                        Text(verbatim: block.text).font(.system(.callout, design: .monospaced)).textSelection(.enabled).padding(8)
                    }
                    .background(.quaternary, in: RoundedRectangle(cornerRadius: 6))
                } else {
                    let lines = block.text.components(separatedBy: "\n")
                    VStack(alignment: .leading, spacing: 4) {
                        ForEach(lines.indices, id: \.self) { line in
                            markdownLine(lines[line])
                        }
                    }
                }
            }
        }
        .environment(\.openURL, OpenURLAction { url in
            Self.allowsLink(url) ? .systemAction : .discarded
        })
    }
}
