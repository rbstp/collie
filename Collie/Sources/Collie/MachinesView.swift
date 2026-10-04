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
                VStack(alignment: .leading, spacing: 2) {
                    MachineName(machine: machine)
                    Text(machine.host).font(.caption).foregroundStyle(.secondary)
                }
            }
            .onDelete { offsets in
                for machine in offsets.map({ app.machines[$0] }) {
                    do {
                        try app.removeMachine(machine)
                    } catch {
                        removeError = describe(error)
                    }
                }
            }
            if let removeError {
                Text(removeError).foregroundStyle(.red)
            }
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

                Section("Pairing code") {
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

    var body: some View {
        Label {
            Text(machine.label)
        } icon: {
            Image(systemName: machine.kind == .linux ? "terminal" : "apple.logo")
                .accessibilityLabel(machine.kind == .linux ? "Linux" : "Mac")
        }
    }
}
