import CollieCore
import Foundation

struct ConversationTool: Identifiable {
    let id: String
    var records: [HistoryMessage]

    var name: String { records.first { $0.role != "tool" }?.tool ?? "Tool result" }
}

struct ConversationRow: Identifiable {
    let id: String
    let message: HistoryMessage?
    var tools: [ConversationTool]

    var recordIDs: [String] { message.map { [$0.id] } ?? tools.flatMap { $0.records.map(\.id) } }

    static func build(_ messages: [HistoryMessage], previous: [Self] = [], searching: Bool = false) -> [Self] {
        if searching {
            return messages.map { Self(id: "message:\($0.id)", message: $0, tools: []) }
        }
        let existing = Dictionary(previous.filter { $0.message == nil }.flatMap { row in
            row.recordIDs.map { ($0, row.id) }
        }, uniquingKeysWith: { first, _ in first })
        var rows: [Self] = []
        var activity: [HistoryMessage] = []
        func flush() {
            guard let first = activity.first else { return }
            let id = activity.lazy.compactMap { existing[$0.id] }.first ?? "activity:\(first.id)"
            var tools: [ConversationTool] = []
            var indices: [String: Int] = [:]
            for record in activity {
                let key = record.callId.map { "call:\($0)" } ?? "record:\(record.id)"
                if let index = indices[key] {
                    tools[index].records.append(record)
                } else {
                    indices[key] = tools.count
                    tools.append(ConversationTool(id: key, records: [record]))
                }
            }
            rows.append(Self(id: id, message: nil, tools: tools))
            activity = []
        }
        for message in messages {
            if message.tool != nil {
                activity.append(message)
            } else {
                flush()
                rows.append(Self(id: "message:\(message.id)", message: message, tools: []))
            }
        }
        flush()
        return rows
    }
}
