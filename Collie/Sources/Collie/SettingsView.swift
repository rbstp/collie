import CollieCore
import SwiftUI

struct SettingsView: View {
    let app: AppModel
    @State private var removeError: String?
    @State private var report: ColdStartReport?
    @State private var keepKeyboard = DevicePrefs.load(from: DevicePrefs.file).keepKeyboard
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
                Section("Prompt") {
                    Toggle("Keep keyboard open after sending", isOn: $keepKeyboard)
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
            .refreshable { await app.refreshNode() }
            .task {
                await app.refreshNode()
                report = app.core?.coldStartReport()
            }
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
