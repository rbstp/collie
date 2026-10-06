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
                Section("Notifications") {
                    LabeledContent("Push", value: app.pushStatus)
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
                var prefs = DevicePrefs.load(from: DevicePrefs.file)
                prefs.keepKeyboard = keep
                prefs.save(to: DevicePrefs.file)
            }
            .onChange(of: historyLines) { _, lines in
                var prefs = DevicePrefs.load(from: DevicePrefs.file)
                prefs.historyLines = lines
                prefs.save(to: DevicePrefs.file)
            }
            .refreshable { await app.refreshNode() }
            .task {
                await app.refreshNode()
                report = app.core?.coldStartReport()
            }
        }
    }

    /// Turning it off needs no authentication.
    private func setWatchDecisions(_ on: Bool) async {
        if on, !(await DeviceOwnerAuthenticator().authenticate(reason: "Allow decisions from Apple Watch")) { return }
        var prefs = DevicePrefs.load(from: DevicePrefs.file)
        prefs.watchDecisions = on
        prefs.save(to: DevicePrefs.file)
        watchDecisions = DevicePrefs.load(from: DevicePrefs.file).watchDecisions
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
            var prefs = DevicePrefs.load(from: DevicePrefs.file)
            prefs.gestures = gestures
            prefs.save(to: DevicePrefs.file)
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
