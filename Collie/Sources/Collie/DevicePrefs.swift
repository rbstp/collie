import Foundation

/// Display choices kept on this device only, in the state dir rather than UserDefaults.
struct DevicePrefs: Codable, Equatable {
    var wrapLines = true
    var keepKeyboard = false
    var gestures = TerminalGestures()
    var fontSize = 11.0
    var dictationLanguage = DictationLanguage.english
    var agentsLayout = AgentsLayout.grid
    var starred: Set<AgentRoute> = []
    var historyLines: UInt16 = 200

    static let historyChoices: [UInt16] = [200, 500, 1000]

    static let file: URL? = try? StateDirectory.prepare().appending(path: "prefs.json")

    static func load(from file: URL?) -> DevicePrefs {
        file.flatMap { try? Data(contentsOf: $0) }.flatMap { try? JSONDecoder().decode(Self.self, from: $0) } ?? DevicePrefs()
    }

    func save(to file: URL?) {
        guard let file, let data = try? JSONEncoder().encode(self) else { return }
        try? data.write(to: file, options: .atomic)
    }
}

extension DevicePrefs {
    /// A file saved before a field existed keeps the fields it has.
    init(from decoder: any Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        let defaults = DevicePrefs()
        wrapLines = try container.decodeIfPresent(Bool.self, forKey: .wrapLines) ?? defaults.wrapLines
        keepKeyboard = try container.decodeIfPresent(Bool.self, forKey: .keepKeyboard) ?? defaults.keepKeyboard
        gestures = (try? container.decodeIfPresent(TerminalGestures.self, forKey: .gestures)) ?? defaults.gestures
        fontSize = try container.decodeIfPresent(Double.self, forKey: .fontSize) ?? defaults.fontSize
        dictationLanguage = (try? container.decodeIfPresent(DictationLanguage.self, forKey: .dictationLanguage)) ?? defaults.dictationLanguage
        agentsLayout = (try? container.decodeIfPresent(AgentsLayout.self, forKey: .agentsLayout)) ?? defaults.agentsLayout
        starred = (try? container.decodeIfPresent(Set<AgentRoute>.self, forKey: .starred)) ?? defaults.starred
        historyLines = try container.decodeIfPresent(UInt16.self, forKey: .historyLines) ?? defaults.historyLines
    }
}

enum AgentsLayout: String, Codable, CaseIterable {
    case grid
    case inbox
    case list

    var label: String {
        switch self {
        case .grid: "Grid"
        case .inbox: "Inbox"
        case .list: "List"
        }
    }

    var icon: String {
        switch self {
        case .grid: "rectangle.grid.2x2"
        case .inbox: "tray"
        case .list: "list.bullet"
        }
    }
}

struct TerminalGestures: Codable, Equatable {
    var doubleTap = GestureAction.paste
    var tripleTap = GestureAction.none
    var pinchResizesText = true
    var swipeSwitchesAgents = true
}

enum GestureAction: String, Codable, CaseIterable {
    case none
    case paste
    case escape

    var label: String {
        switch self {
        case .none: "None"
        case .paste: "Paste into prompt"
        case .escape: "Send Esc"
        }
    }
}
