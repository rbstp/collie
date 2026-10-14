import Foundation
import SwiftUI

/// Display choices kept on this device only, in the state dir rather than UserDefaults.
struct DevicePrefs: StateFile, Equatable {
    var wrapLines = true
    var keepKeyboard = false
    var keysShown = false
    var gestures = TerminalGestures()
    var fontSize = 11.0
    var dictationLanguage = DictationLanguage.english
    var agentsLayout = AgentsLayout.grid
    var historyLines: UInt16 = 200
    var defaultSessionView = SessionView.chat
    var watchDecisions = false
    var doneAlerts = true
    /// Nil until the user picks it, so a phone that never did leaves the Macs' setting alone.
    var archiveOnMerge: Bool?
    var accent = AppAccent.blue
    /// Per machine id: the canonical folder collied returned when it was saved.
    var taskBases: [String: String] = [:]
    var taskUltracode: [String: Bool] = [:]

    static let historyChoices: [UInt16] = [200, 500, 1000]

    static let file: URL? = try? StateDirectory.prepare().appending(path: "prefs.json")

    static func update(in file: URL?, _ change: (inout DevicePrefs) -> Void) {
        var prefs = load(from: file)
        change(&prefs)
        prefs.save(to: file)
    }
}

extension DevicePrefs {
    /// Turning it on needs the device owner, turning it off needs nothing. Nil when not authenticated.
    @MainActor
    static func setWatchDecisions(_ on: Bool, in file: URL?, auth: any Authenticator) async -> Bool? {
        if on, !(await auth.authenticate(reason: "Allow decisions from Apple Watch")) { return nil }
        update(in: file) { $0.watchDecisions = on }
        return load(from: file).watchDecisions
    }

    /// Another watch needs a fresh authenticated opt-in before it can decide.
    @MainActor
    static func turnOffWatchDecisions(in file: URL?) {
        var prefs = load(from: file)
        if prefs.watchDecisions {
            prefs.watchDecisions = false
            prefs.save(to: file)
        }
    }

    /// A file saved before a field existed keeps the fields it has.
    init(from decoder: any Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        let defaults = DevicePrefs()
        wrapLines = (try? container.decodeIfPresent(Bool.self, forKey: .wrapLines)) ?? defaults.wrapLines
        keepKeyboard = (try? container.decodeIfPresent(Bool.self, forKey: .keepKeyboard)) ?? defaults.keepKeyboard
        keysShown = (try? container.decodeIfPresent(Bool.self, forKey: .keysShown)) ?? defaults.keysShown
        gestures = (try? container.decodeIfPresent(TerminalGestures.self, forKey: .gestures)) ?? defaults.gestures
        fontSize = (try? container.decodeIfPresent(Double.self, forKey: .fontSize)) ?? defaults.fontSize
        dictationLanguage = (try? container.decodeIfPresent(DictationLanguage.self, forKey: .dictationLanguage)) ?? defaults.dictationLanguage
        agentsLayout = (try? container.decodeIfPresent(AgentsLayout.self, forKey: .agentsLayout)) ?? defaults.agentsLayout
        historyLines = (try? container.decodeIfPresent(UInt16.self, forKey: .historyLines)) ?? defaults.historyLines
        defaultSessionView = (try? container.decodeIfPresent(SessionView.self, forKey: .defaultSessionView)) ?? defaults.defaultSessionView
        watchDecisions = (try? container.decodeIfPresent(Bool.self, forKey: .watchDecisions)) ?? defaults.watchDecisions
        doneAlerts = (try? container.decodeIfPresent(Bool.self, forKey: .doneAlerts)) ?? defaults.doneAlerts
        archiveOnMerge = try? container.decodeIfPresent(Bool.self, forKey: .archiveOnMerge)
        accent = (try? container.decodeIfPresent(AppAccent.self, forKey: .accent)) ?? defaults.accent
        taskBases = (try? container.decodeIfPresent([String: String].self, forKey: .taskBases)) ?? defaults.taskBases
        taskUltracode = (try? container.decodeIfPresent([String: Bool].self, forKey: .taskUltracode)) ?? defaults.taskUltracode
    }

    /// Kept only once collied has listed it, in the canonical form it returned.
    @MainActor
    static func setTaskBase(_ path: String, machineId: String, core: any AgentCore, in file: URL?) async throws -> String {
        let listed = try await core.taskFolders(machineId: machineId, path: path)
        update(in: file) { $0.taskBases[machineId] = listed.path }
        return listed.path
    }

    static func forgetTaskBase(machineId: String, in file: URL?) {
        guard load(from: file).taskBases[machineId] != nil else { return }
        update(in: file) { $0.taskBases[machineId] = nil }
    }

    static func forgetMachine(machineId: String, in file: URL?) {
        let prefs = load(from: file)
        guard prefs.taskBases[machineId] != nil || prefs.taskUltracode[machineId] != nil else { return }
        update(in: file) {
            $0.taskBases[machineId] = nil
            $0.taskUltracode[machineId] = nil
        }
    }
}

enum AppAccent: String, Codable, CaseIterable {
    case blue, orange, green, red, gold

    var label: String {
        switch self {
        case .blue: "Blue"
        case .orange: "Orange"
        case .green: "Green"
        case .red: "Red"
        case .gold: "Dark Gold"
        }
    }

    var color: Color {
        switch self {
        case .blue: .blue
        case .orange: .orange
        case .green: .green
        case .red: .red
        case .gold:
            Color(uiColor: UIColor { traits in
                traits.userInterfaceStyle == .dark
                    ? UIColor(red: 0.78, green: 0.62, blue: 0.25, alpha: 1)
                    : UIColor(red: 0.50, green: 0.36, blue: 0.08, alpha: 1)
            })
        }
    }
}

enum SessionView: String, Codable, CaseIterable {
    case chat
    case terminal

    var label: String {
        switch self {
        case .chat: "Chat"
        case .terminal: "Terminal"
        }
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
