import CollieCore
import Foundation

struct ConversationTool: Identifiable {
    let id: String
    var records: [HistoryMessage]
    var steps: [HistoryMessage] = []

    var name: String { records.first { $0.role != "tool" }?.tool ?? (records.isEmpty ? "Subagent" : "Tool result") }

    var subagent: [ConversationTool] { ConversationRow.group(steps) }
}

struct ConversationRow: Identifiable {
    let id: String
    let message: HistoryMessage?
    var tools: [ConversationTool]

    var recordIDs: [String] { message.map { [$0.id] } ?? tools.flatMap { ($0.records + $0.steps).map(\.id) } }

    var event: Bool { message?.role == "event" }

    var subagentSteps: Int { tools.reduce(0) { $0 + $1.subagent.count } }

    static func group(_ records: [HistoryMessage]) -> [ConversationTool] {
        var tools: [ConversationTool] = []
        var indices: [String: Int] = [:]
        for record in records {
            let key = record.callId.map { "call:\($0)" } ?? "record:\(record.id)"
            if let index = indices[key] {
                tools[index].records.append(record)
            } else {
                indices[key] = tools.count
                tools.append(ConversationTool(id: key, records: [record]))
            }
        }
        return tools
    }

    static func build(_ messages: [HistoryMessage], previous: [Self] = [], searching: Bool = false) -> [Self] {
        if searching {
            return messages.map { Self(id: "message:\($0.id)", message: $0, tools: []) }
        }
        let existing = Dictionary(previous.filter { $0.message == nil }.flatMap { row in
            row.recordIDs.map { ($0, row.id) }
        }, uniquingKeysWith: { first, _ in first })
        var rows: [Self] = []
        var activity: [HistoryMessage] = []
        var calls: [String: (row: Int, tool: Int)] = [:]
        func flush() {
            guard let first = activity.first else { return }
            let id = activity.lazy.compactMap { existing[$0.id] }.first ?? "activity:\(first.id)"
            var tools: [ConversationTool] = []
            var indices: [String: Int] = [:]
            for record in activity {
                let call = record.parentCallId ?? record.callId
                let key = call.map { "call:\($0)" } ?? "record:\(record.id)"
                let index = indices[key] ?? tools.count
                if index == tools.count {
                    indices[key] = index
                    tools.append(ConversationTool(id: key, records: []))
                }
                if record.parentCallId == nil { tools[index].records.append(record) } else { tools[index].steps.append(record) }
                if let call { calls[call] = (rows.count, index) }
            }
            rows.append(Self(id: id, message: nil, tools: tools))
            activity = []
        }
        for message in messages {
            if let parent = message.parentCallId, let at = calls[parent] {
                rows[at.row].tools[at.tool].steps.append(message)
            } else if message.role == "tool", message.callId == nil, message.parentCallId == nil {
                flush()
                activity = [message]
                flush()
            } else if message.tool != nil || message.parentCallId != nil {
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
