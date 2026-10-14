import CollieCore
import SwiftUI
import UIKit

@MainActor
@Observable
final class ConversationModel {
    private(set) var messages: [HistoryMessage] = []
    private(set) var rows: [ConversationRow] = []
    private(set) var session: String?
    private(set) var available: Bool?
    private(set) var unavailableReason: String?
    private(set) var before: String?
    private(set) var after: String?
    private(set) var loading = false
    private(set) var loadingOlder = false
    private(set) var omitted = false
    private(set) var error: String?
    private(set) var query = ""
    private(set) var more = false
    private var generation = 0

    typealias Load = @MainActor (String?, String?, String?) async throws -> HistoryPage

    func stop() {
        generation += 1
        loading = false
        loadingOlder = false
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
            rows = []
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
        loadingOlder = older
        defer {
            if ticket == generation {
                loading = false
                loadingOlder = false
            }
        }
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
            rows = []
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
            rows = ConversationRow.build(messages, previous: rows, searching: !query.isEmpty)
        }
        if older || replacing || after == nil { before = page.before }
        if !older || replacing { after = page.after }
        omitted = omitted || page.omitted
        more = page.more
    }
}

struct ConversationView: View {
    @Bindable var model: ConversationModel
    @Binding var searching: Bool
    let load: ConversationModel.Load
    let terminal: () -> Void
    @State private var search = ""
    @State private var submitted = ""
    @State private var visibleID: String?
    @State private var atBottom = true
    @State private var selectedMatch: String?
    @FocusState private var searchFocused: Bool
    @Environment(\.scenePhase) private var scenePhase

    var body: some View {
        VStack(spacing: 0) {
            if searching { searchBar }
            if let error = model.error {
                Text(error).font(.footnote).foregroundStyle(.orange).padding(.horizontal)
            }
            if model.available == false {
                ContentUnavailableView {
                    Label("History unavailable", systemImage: "text.bubble")
                } description: {
                    Text(model.unavailableReason ?? "This session has no readable Codex or Claude Code transcript yet. Older daemons need an update for Chat.")
                } actions: {
                    Button("Open Terminal", action: terminal).buttonStyle(.glass)
                }
            } else {
                history
            }
        }
        .onChange(of: searching) { _, value in
            searchFocused = value
            if !value { search = ""; submitted = ""; selectedMatch = nil }
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
                    } else { await model.fetch(load: load) }
                } else {
                    await model.fetch(load: load)
                }
                try? await Task.sleep(for: .milliseconds(model.more || !submitted.isEmpty && model.before != nil ? 150 : 1000))
            }
        }
    }

    private var searchBar: some View {
        GlassEffectContainer(spacing: 8) {
            HStack(spacing: 8) {
                HStack {
                    Image(systemName: "magnifyingglass").foregroundStyle(.secondary)
                    TextField("Search conversation", text: $search)
                        .focused($searchFocused)
                        .submitLabel(.search)
                        .onSubmit {
                            submitted = ConversationModel.boundedQuery(search)
                            selectedMatch = nil
                            searchFocused = false
                        }
                }
                .padding(12)
                .glassEffect(.regular.interactive(), in: .capsule)
                Button("Close search", systemImage: "xmark") { searching = false }
                    .labelStyle(.iconOnly).buttonStyle(.glass).controlSize(.large)
            }
        }
        .padding(.horizontal)
        .padding(.vertical, 6)
    }

    private var history: some View {
        ScrollViewReader { proxy in
            VStack(spacing: 0) {
                if !submitted.isEmpty {
                    HStack {
                        Text("\(model.messages.count) matches\(model.before != nil ? ", searching…" : "")")
                            .foregroundStyle(.secondary)
                        Spacer()
                        if !model.messages.isEmpty {
                            GlassEffectContainer(spacing: 8) {
                                HStack(spacing: 8) {
                                    Button("Previous match", systemImage: "chevron.up") { navigate(-1, proxy: proxy) }
                                    Button("Next match", systemImage: "chevron.down") { navigate(1, proxy: proxy) }
                                }
                                .labelStyle(.iconOnly).buttonStyle(.glass)
                            }
                        }
                    }
                    .font(.caption)
                    .padding(.horizontal)
                    .padding(.bottom, 6)
                }
                ScrollView {
                    LazyVStack(alignment: .leading, spacing: 20) {
                        if model.before != nil && submitted.isEmpty {
                            Button {
                                Task { await model.fetch(older: true, load: load) }
                            } label: {
                                HStack(spacing: 8) {
                                    if model.loadingOlder { ProgressView().controlSize(.small) }
                                    Text(model.loadingOlder ? "Loading older messages…" : "Load older messages")
                                }
                            }
                                .font(.footnote).buttonStyle(.glass)
                                .disabled(model.loading)
                                .frame(maxWidth: .infinity)
                        }
                        if model.omitted {
                            Text("Some transcript records could not be displayed.")
                                .font(.footnote).foregroundStyle(.secondary)
                        }
                        if model.loading && model.available == nil { ProgressView().frame(maxWidth: .infinity) }
                        if model.available == true && model.messages.isEmpty {
                            Text(model.before != nil ? (submitted.isEmpty ? "Load older messages to continue." : "Searching history…") : submitted.isEmpty ? "No messages yet." : "No matches.")
                                .foregroundStyle(.secondary)
                        }
                        ForEach(model.rows) { row in
                            ConversationRowView(row: row, query: submitted).id(row.id)
                        }
                        Color.clear.frame(height: 1).id("bottom")
                    }
                    .scrollTargetLayout()
                    .padding()
                }
                .scrollDismissesKeyboard(.interactively)
                .scrollPosition(id: $visibleID, anchor: .top)
                .defaultScrollAnchor(.bottom, for: .initialOffset)
                .onScrollGeometryChange(for: Bool.self) { geo in
                    geo.contentOffset.y + geo.containerSize.height >= geo.contentSize.height - 60
                } action: { _, bottom in atBottom = bottom }
                .onChange(of: model.session) { _, _ in
                    visibleID = nil
                    selectedMatch = nil
                    proxy.scrollTo("bottom", anchor: .bottom)
                }
                .onChange(of: model.messages.last?.id) { _, _ in
                    if atBottom && submitted.isEmpty { proxy.scrollTo("bottom", anchor: .bottom) }
                }
                .overlay(alignment: .bottomTrailing) {
                    if !atBottom && submitted.isEmpty {
                        Button("Latest messages", systemImage: "arrow.down") { proxy.scrollTo("bottom", anchor: .bottom) }
                            .labelStyle(.iconOnly).buttonStyle(.glass).controlSize(.large)
                            .padding(12)
                    }
                }
            }
        }
    }

    private func navigate(_ direction: Int, proxy: ScrollViewProxy) {
        guard !model.messages.isEmpty else { return }
        let current = model.messages.firstIndex { $0.id == selectedMatch } ?? (direction < 0 ? model.messages.count : -1)
        let index = (current + direction + model.messages.count) % model.messages.count
        selectedMatch = model.messages[index].id
        proxy.scrollTo("message:\(model.messages[index].id)", anchor: .top)
    }
}

struct ConversationRowView: View {
    let row: ConversationRow
    let query: String

    var body: some View {
        Group {
            if let message = row.message {
                ConversationMessage(message: message, query: query)
            } else {
                ConversationActivity(tools: row.tools, steps: row.subagentSteps)
            }
        }
    }
}

private struct ConversationActivity: View {
    let tools: [ConversationTool]
    let steps: Int
    @State private var expanded = false

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Button {
                expanded.toggle()
            } label: {
                HStack(spacing: 8) {
                    Image(systemName: "wrench.and.screwdriver")
                    Text("Activity · \(tools.count) \(tools.count == 1 ? "call" : "calls")\(steps == 0 ? "" : " · \(steps) subagent \(steps == 1 ? "step" : "steps")")")
                    Image(systemName: expanded ? "chevron.up" : "chevron.down").font(.caption2)
                }
                .font(.footnote)
            }
            .buttonStyle(.glass)
            .accessibilityValue(expanded ? "Expanded" : "Collapsed")
            if expanded {
                LazyVStack(alignment: .leading, spacing: 12) {
                    ForEach(tools) { tool in
                        DisclosureGroup {
                            ForEach(tool.records, id: \.id) { record in
                                ConversationToolRecord(message: record, query: "")
                            }
                            if !tool.steps.isEmpty {
                                LazyVStack(alignment: .leading, spacing: 12) {
                                    ForEach(tool.subagent) { step in
                                        if step.records.contains(where: { $0.tool != nil }) {
                                            DisclosureGroup {
                                                ForEach(step.records, id: \.id) { record in
                                                    ConversationToolRecord(message: record, query: "")
                                                }
                                            } label: {
                                                Text(verbatim: step.name).font(.callout)
                                            }
                                        } else {
                                            ForEach(step.records, id: \.id) { record in
                                                ConversationToolRecord(message: record, query: "")
                                            }
                                        }
                                    }
                                }
                                .padding(.leading, 12)
                            }
                        } label: {
                            Text(verbatim: tool.name).font(.callout.weight(.medium))
                        }
                    }
                }
                .padding(.leading, 12)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}

private struct ConversationMessage: View {
    let message: HistoryMessage
    let query: String
    @Environment(AgentModel.self) private var agent: AgentModel?

    private var user: Bool { message.role == "user" && message.parentCallId == nil }

    var body: some View {
        HStack(alignment: .top) {
            if user { Spacer(minLength: 28) }
            VStack(alignment: .leading, spacing: 6) {
                if user || message.parentCallId != nil {
                    Text(user ? "You" : "Subagent").font(.caption.weight(.medium)).foregroundStyle(.secondary)
                }
                if message.tool != nil {
                    Text(verbatim: message.tool ?? "Tool").font(.callout.weight(.semibold))
                    ConversationToolRecord(message: message, query: query)
                } else {
                    ConversationMarkdown(text: message.text, query: query)
                    ConversationTruncation(truncated: message.truncated)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(user ? 12 : 0)
            .background {
                if user {
                    RoundedRectangle(cornerRadius: 18).fill(.tint.opacity(0.1))
                }
            }
        }
        .contextMenu {
            Button("Copy", systemImage: "doc.on.doc") { UIPasteboard.general.string = message.text }
            if let agent {
                Button("Quote", systemImage: "text.quote") { agent.quote(message.text) }
            }
        }
    }
}

private struct ConversationToolRecord: View {
    let message: HistoryMessage
    let query: String
    @Environment(AgentModel.self) private var agent: AgentModel?

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack {
                Text(message.role == "tool" ? "Result" : message.tool != nil ? "Input" : message.role == "user" ? "Subagent prompt" : "Subagent reply")
                    .font(.caption).foregroundStyle(.secondary)
                Spacer()
                if let agent {
                    Button("Quote", systemImage: "text.quote") { agent.quote(message.text) }
                        .labelStyle(.iconOnly).buttonStyle(.glass).controlSize(.small)
                }
                Button("Copy", systemImage: "doc.on.doc") { UIPasteboard.general.string = message.text }
                    .labelStyle(.iconOnly).buttonStyle(.glass).controlSize(.small)
            }
            if message.tool == nil {
                ConversationMarkdown(text: message.text, query: query)
            } else {
                ScrollView(.horizontal) {
                    Text(ConversationMarkdown.highlighted(AttributedString(message.text), query: query))
                        .font(.system(.caption, design: .monospaced)).textSelection(.enabled)
                }
            }
            ConversationTruncation(truncated: message.truncated)
        }
        .padding(.vertical, 6)
    }
}

private struct ConversationTruncation: View {
    let truncated: Bool

    var body: some View {
        if truncated {
            Text("Long content shortened for display. Search includes the full recorded text within the read limit.")
                .font(.caption).foregroundStyle(.secondary)
        }
    }
}

struct ConversationMarkdown: View {
    let text: String
    var query: String = ""
    @Environment(AgentModel.self) private var agent: AgentModel?

    static func allowsLink(_ url: URL) -> Bool {
        guard let scheme = url.scheme?.lowercased(), ["http", "https"].contains(scheme) else { return false }
        return url.host?.isEmpty == false
    }

    static func highlighted(_ text: AttributedString, query: String) -> AttributedString {
        guard !query.isEmpty else { return text }
        var result = text
        let plain = String(text.characters)
        var start = plain.startIndex
        while start < plain.endIndex,
              let match = plain.range(of: query, options: .caseInsensitive, range: start..<plain.endIndex) {
            guard let lower = AttributedString.Index(match.lowerBound, within: result),
                  let upper = AttributedString.Index(match.upperBound, within: result) else { break }
            result[lower..<upper].backgroundColor = .yellow.opacity(0.3)
            start = match.upperBound
        }
        return result
    }

    static func blocks(_ text: String) -> [(code: Bool, text: String)] {
        var result: [(Bool, String)] = []
        var code = false
        var lines: [String] = []
        for line in text.components(separatedBy: "\n") {
            if line.trimmingCharacters(in: .whitespaces).hasPrefix("```") {
                if !lines.isEmpty { result.append((code, lines.joined(separator: "\n"))); lines = [] }
                code.toggle()
            } else if !code && line.trimmingCharacters(in: .whitespaces).isEmpty {
                if !lines.isEmpty { result.append((false, lines.joined(separator: "\n"))); lines = [] }
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
        return Text(Self.highlighted(rendered, query: query))
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
                    VStack(alignment: .leading, spacing: 6) {
                        HStack {
                            Text("Code").font(.caption).foregroundStyle(.secondary)
                            Spacer()
                            if let agent {
                                Button("Quote code", systemImage: "text.quote") { agent.quote(block.text) }
                                    .labelStyle(.iconOnly).buttonStyle(.glass).controlSize(.small)
                            }
                            Button("Copy code", systemImage: "doc.on.doc") { UIPasteboard.general.string = block.text }
                                .labelStyle(.iconOnly).buttonStyle(.glass).controlSize(.small)
                        }
                        ScrollView(.horizontal) {
                            Text(Self.highlighted(AttributedString(block.text), query: query))
                                .font(.system(.callout, design: .monospaced)).textSelection(.enabled)
                        }
                    }
                    .padding(10)
                    .background(.quaternary, in: RoundedRectangle(cornerRadius: 10))
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
