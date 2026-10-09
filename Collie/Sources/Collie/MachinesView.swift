import AVFoundation
import CollieCore
import SwiftUI

struct MachinesView: View {
    let app: AppModel
    @State private var adding = false
    @State private var removeError: String?

    var body: some View {
        NavigationStack {
            MachinesList(app: app, removeError: $removeError)
                .navigationTitle("Machines")
                .toolbar {
                    Button("Add machine", systemImage: "plus") { adding = true }
                }
                .sheet(isPresented: $adding, onDismiss: app.reloadMachines) {
                    PairView(app: app)
                }
        }
    }
}

struct MachinesList: View {
    let app: AppModel
    @Binding var removeError: String?

    var body: some View {
        List {
            if app.machines.isEmpty {
                Text("No machines paired. Run `collied pair` on the machine, then add it here.")
                    .foregroundStyle(.secondary)
            }
            ForEach(app.machines, id: \.id) { machine in
                NavigationLink {
                    MachineSettingsView(app: app, machine: machine)
                } label: {
                    TimelineView(.periodic(from: .now, by: 2)) { _ in
                        let link = app.core?.cachedFlock(machineId: machine.id)?.link
                        let reachable = link.map { [.connected, .connecting].contains($0) } ?? true
                        VStack(alignment: .leading, spacing: 2) {
                            MachineName(machine: machine)
                                .foregroundStyle(reachable ? .primary : .secondary)
                                .tint(reachable ? nil : .secondary)
                            Text(machine.host).font(.caption).foregroundStyle(.secondary)
                        }
                    }
                }
            }
            .onDelete { offsets in
                let removed = offsets.map { app.machines[$0] }
                removed.forEach(app.hideMachine)
                removeError = nil
                Task {
                    for machine in removed {
                        do {
                            if try await !app.removeMachine(machine) {
                                removeError = "\(machine.label) did not confirm it removed this phone. If collied peers list there still shows it, run collied peers revoke to stop its notifications."
                            }
                        } catch {
                            removeError = describe(error)
                        }
                    }
                }
            }
            if let removeError {
                Text(removeError).foregroundStyle(.red)
            }
        }
    }
}

struct MachineSettingsView: View {
    let app: AppModel
    let machine: Machine
    @State private var base = ""
    @State private var saved: String?
    @State private var roots: [String] = []
    @State private var saving = false
    @State private var error: String?

    private var typed: String { base.trimmingCharacters(in: .whitespacesAndNewlines) }

    var body: some View {
        Form {
            Section {
                TextField("/path/to/folder", text: $base)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .font(.body.monospaced())
                ForEach(roots, id: \.self) { root in
                    Button {
                        base = root.hasSuffix("/") ? root : root + "/"
                    } label: {
                        Label(root, systemImage: "folder").font(.footnote.monospaced())
                    }
                    .foregroundStyle(.primary)
                }
                Button {
                    Task { await save() }
                } label: {
                    if saving {
                        HStack { ProgressView(); Text("Checking the folder…") }
                    } else {
                        Text("Save")
                    }
                }
                .disabled(saving || !typed.hasPrefix("/") || typed == saved || app.core == nil)
                if saved != nil {
                    Button("Clear", role: .destructive) {
                        DevicePrefs.forgetTaskBase(machineId: machine.id, in: DevicePrefs.file)
                        saved = nil
                        base = roots.first.map { $0.hasSuffix("/") ? $0 : $0 + "/" } ?? ""
                    }
                }
                if let error {
                    Text(error).foregroundStyle(.red)
                }
            } header: {
                Text("Base folder")
            } footer: {
                VStack(alignment: .leading, spacing: 4) {
                    if !typed.isEmpty, !typed.hasPrefix("/") {
                        Text("Type the full path, for example \(roots.first ?? "/Users/you")/git: ~ is not expanded.")
                            .foregroundStyle(.orange)
                    }
                    Text("New Task completes folder names and lists the folders inside this folder. It must be inside one of the machine's task roots.")
                }
            }
        }
        .navigationTitle(machine.label)
        .task { await load() }
    }

    private func load() async {
        saved = DevicePrefs.load(from: DevicePrefs.file).taskBases[machine.id]
        base = saved ?? ""
        guard let core = app.core else { return }
        do {
            roots = try await core.taskOptions(machineId: machine.id).roots
            if base.isEmpty, let root = roots.first { base = root.hasSuffix("/") ? root : root + "/" }
        } catch {
            self.error = AgentModel.message(for: error)
        }
    }

    private func save() async {
        guard let core = app.core else { return }
        saving = true
        error = nil
        defer { saving = false }
        do {
            saved = try await DevicePrefs.setTaskBase(typed, machineId: machine.id, core: core, in: DevicePrefs.file)
            base = saved ?? base
        } catch {
            self.error = AgentModel.message(for: error)
        }
    }
}

struct PairView: View {
    let app: AppModel
    @State private var model = PairingModel()
    @State private var scanning = false
    @State private var scanError: String?
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    TextField("Device name", text: $model.deviceLabel)
                        .textInputAutocapitalization(.words)
                } header: {
                    Text("This phone")
                } footer: {
                    Text("Shown on the machine in its list of paired devices.")
                }

                Section {
                    if QRScannerView.isSupported {
                        Button("Scan QR code", systemImage: "qrcode.viewfinder") {
                            Task { await startScanning() }
                        }
                    }
                    if let scanError {
                        Label(scanError, systemImage: "camera.badge.ellipsis")
                            .foregroundStyle(.red)
                    }
                    TextField("collie://pair#…", text: $model.invite, axis: .vertical)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        .font(.footnote.monospaced())
                    PasteButton(payloadType: String.self) { strings in
                        if let first = strings.first {
                            model.invite = first.trimmingCharacters(in: .whitespacesAndNewlines)
                        }
                    }
                } header: {
                    Text("Pairing code")
                } footer: {
                    Text("On the computer, run collied setup, or collied pair if it is already set up, and scan the QR code it shows.")
                }

                Section {
                    Button {
                        Task { await pair() }
                    } label: {
                        if model.pairing {
                            HStack {
                                ProgressView()
                                Text("Confirm the pairing on the machine…")
                            }
                        } else {
                            Text("Pair")
                        }
                    }
                    .disabled(!model.canPair)
                    if let error = model.error {
                        Label(error, systemImage: "xmark.octagon")
                            .foregroundStyle(.red)
                    }
                }
            }
            .navigationTitle("Add machine")
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { dismiss() }
                }
            }
            .fullScreenCover(isPresented: $scanning) {
                NavigationStack {
                    QRScannerView(
                        onScan: { payload in
                            scanning = false
                            model.invite = payload
                            Task { await pair() }
                        },
                        onError: { message in
                            scanning = false
                            scanError = message
                        }
                    )
                    .ignoresSafeArea()
                    .toolbar {
                        ToolbarItem(placement: .cancellationAction) {
                            Button("Cancel") { scanning = false }
                        }
                    }
                }
            }
        }
    }

    private func startScanning() async {
        scanError = nil
        if AVCaptureDevice.authorizationStatus(for: .video) == .notDetermined {
            _ = await AVCaptureDevice.requestAccess(for: .video)
        }
        if QRScannerView.isAvailable {
            scanning = true
        } else {
            scanError = "Camera access is off for collie. Allow it in Settings > collie, or paste the pairing link."
        }
    }

    private func pair() async {
        await model.pair(core: app.core)
        if let machine = model.paired {
            app.machinePaired(machine)
            dismiss()
        }
    }
}

struct MachineName: View {
    let machine: Machine
    // SF Symbols has no Linux glyph; Tux is a drawn template image, sized like the symbol.
    @ScaledMetric(relativeTo: .body) private var tuxSize: CGFloat = 18

    var body: some View {
        Label {
            Text(machine.label)
        } icon: {
            Group {
                if machine.kind == .linux {
                    Image("Tux").resizable().scaledToFit().frame(width: tuxSize, height: tuxSize)
                } else {
                    Image(systemName: "apple.logo")
                }
            }
            .accessibilityLabel(machine.kind == .linux ? "Linux" : "Mac")
        }
    }
}
