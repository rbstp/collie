import CollieCore
import SwiftUI
import WatchConnectivity

struct SettingsView: View {
    let app: AppModel
    @State private var removeError: String?
    @State private var report: ColdStartReport?
    @State private var keepKeyboard = DevicePrefs.load(from: DevicePrefs.file).keepKeyboard
    @State private var historyLines = DevicePrefs.load(from: DevicePrefs.file).historyLines
    @State private var watchDecisions = DevicePrefs.load(from: DevicePrefs.file).watchDecisions
    @State private var doneAlerts = DevicePrefs.load(from: DevicePrefs.file).doneAlerts
    private let build = buildInfo()

    var body: some View {
        NavigationStack {
            List {
                Section("Tailnet") {
                    LabeledContent("State", value: app.node.map { $0.backendState.label } ?? "unknown")
                    LabeledContent("Device", value: app.node?.selfDnsName ?? "-")
                    LabeledContent("Signed in as", value: app.node?.loginName ?? "-")
                    if let error = app.nodeError {
                        Text(error).foregroundStyle(.red)
                    }
                }
                Section {
                    LabeledContent("Push", value: app.pushStatus)
                    Toggle("Notify when an agent finishes", isOn: $doneAlerts)
                } header: {
                    Text("Notifications")
                } footer: {
                    Text("An alert when an agent finishes a turn of 30 seconds or more, not when it stops for an approval.")
                }
                if watchDecisions || (WCSession.isSupported() && WCSession.default.isPaired) {
                    Section {
                        Toggle(
                            "Decide from Apple Watch",
                            isOn: Binding(get: { watchDecisions }, set: { on in Task { await setWatchDecisions(on) } })
                        )
                    } header: {
                        Text("Apple Watch")
                    } footer: {
                        Text("Off by default: the watch shows approvals read-only. When on, the watch can approve, deny or answer a menu while it is unlocked and on your wrist. Turning it on asks for Face ID or the passcode.")
                    }
                }
                Section("Prompt") {
                    NavigationLink("Saved prompts") { PromptShortcutsView() }
                    Toggle("Keep keyboard open after sending", isOn: $keepKeyboard)
                }
                Section("Terminal") {
                    Picker("History", selection: $historyLines) {
                        ForEach(DevicePrefs.historyChoices, id: \.self) { Text("\($0) lines") }
                    }
                    NavigationLink("Gestures") { GesturesView() }
                }
                Section("Machines") {
                    NavigationLink("Paired machines (\(app.machines.count))") {
                        MachinesList(app: app, removeError: $removeError)
                            .navigationTitle("Machines")
                    }
                }
                Section("Cold start") {
                    if let report {
                        LabeledContent("Node created", value: "\(report.nodeCreatedMs) ms")
                        LabeledContent("Started", value: "\(report.startedMs) ms")
                        LabeledContent("Login URL or running", value: report.settledMs.map { "\($0) ms" } ?? "timeout")
                        LabeledContent("Backend state", value: report.backendState.label)
                        LabeledContent("Login URL issued", value: report.authUrlPresent ? "yes" : "no")
                        LabeledContent("Status polls", value: "\(report.statusPolls)")
                    } else {
                        Text("Measured when the tailnet node first starts.").foregroundStyle(.secondary)
                    }
                }
                Section("Build") {
                    LabeledContent("Core", value: build.coreVersion)
                    LabeledContent("Protocol", value: "\(protocolVersion())")
                    LabeledContent("Go", value: build.go)
                    LabeledContent("Rust", value: build.rustc)
                    LabeledContent("Target", value: build.target)
                }
            }
            .navigationTitle("Settings")
            .onChange(of: keepKeyboard) { _, keep in
                DevicePrefs.update(in: DevicePrefs.file) { $0.keepKeyboard = keep }
            }
            .onChange(of: historyLines) { _, lines in
                DevicePrefs.update(in: DevicePrefs.file) { $0.historyLines = lines }
            }
            .onChange(of: doneAlerts) { _, on in app.setDoneAlerts(on) }
            .refreshable { await app.refreshNode() }
            .task {
                await app.refreshNode()
                report = app.core?.coldStartReport()
            }
        }
    }

    private func setWatchDecisions(_ on: Bool) async {
        guard let allowed = await DevicePrefs.setWatchDecisions(on, in: DevicePrefs.file, auth: DeviceOwnerAuthenticator()) else { return }
        watchDecisions = allowed
        app.publishWatchState()
    }
}

private struct GesturesView: View {
    @State private var gestures = DevicePrefs.load(from: DevicePrefs.file).gestures

    var body: some View {
        List {
            Section {
                Picker("Double-tap", selection: $gestures.doubleTap) {
                    ForEach(GestureAction.allCases, id: \.self) { Text($0.label) }
                }
                Picker("Triple-tap", selection: $gestures.tripleTap) {
                    ForEach(GestureAction.allCases, id: \.self) { Text($0.label) }
                }
                Toggle("Pinch to resize text", isOn: $gestures.pinchResizesText)
                Toggle("Swipe sideways to switch agents", isOn: $gestures.swipeSwitchesAgents)
            } footer: {
                Text("Paste only fills the prompt field; it never sends. iOS asks before each paste unless Paste from Other Apps is set to Allow in the Settings app under collie. Swipe left for the next agent in the list and right for the previous one; it works only while lines wrap, since otherwise a sideways swipe scrolls the terminal, and not while a file uploads or a prompt is sending. Gestures are off while text is selected.")
            }
        }
        .navigationTitle("Gestures")
        .onChange(of: gestures) { _, gestures in
            DevicePrefs.update(in: DevicePrefs.file) { $0.gestures = gestures }
        }
    }
}

extension TailnetState {
    var label: String {
        switch self {
        case .notStarted: "not started"
        case .noState: "no state"
        case .needsLogin: "needs login"
        case .needsMachineAuth: "needs approval"
        case .stopped: "stopped"
        case .starting: "starting"
        case .running: "running"
        case .unknown: "unknown"
        }
    }
}
