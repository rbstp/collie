import AVFoundation
import CollieCore
import SwiftUI
import UIKit

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
    @State private var renaming: Machine?

    var body: some View {
        List {
            if app.machines.isEmpty {
                Text("No machines paired. Run `collied pair` on the machine, then add it here.")
                    .foregroundStyle(.secondary)
            }
            ForEach(app.machines, id: \.id) { machine in
                TimelineView(.periodic(from: .now, by: 2)) { _ in
                    MachineRow(machine: machine, flock: app.core?.cachedFlock(machineId: machine.id)) {
                        MachineSettingsView(app: app, machine: machine)
                    }
                }
                .contextMenu {
                    Button("Rename", systemImage: "pencil") { renaming = machine }
                }
                .listRowInsets(EdgeInsets(top: 4, leading: 16, bottom: 4, trailing: 16))
                .listRowBackground(Color.clear)
                .listRowSeparator(.hidden)
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
        .listStyle(.plain)
        .scrollContentBackground(.hidden)
        .renameMachine($renaming, app: app)
    }
}

struct MachineRow<Destination: View>: View {
    let machine: Machine
    let flock: MachineFlock?
    @ViewBuilder let destination: () -> Destination

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            NavigationLink(destination: destination) {
                VStack(alignment: .leading, spacing: 2) {
                    let reachable = flock.map { [.connected, .connecting].contains($0.link) } ?? true
                    HStack(spacing: 8) {
                        MachineName(machine: machine)
                            .font(.body)
                            .lineLimit(1)
                            .minimumScaleFactor(0.85)
                            .foregroundStyle(reachable ? .primary : .secondary)
                            .tint(reachable ? nil : .secondary)
                        Spacer(minLength: 0)
                        if let link = flock?.link {
                            StatusPill(state: link == .connected ? .done : .idle, title: link.label)
                        }
                        Image(systemName: "chevron.right")
                            .font(.caption.weight(.semibold))
                            .foregroundStyle(.secondary)
                    }
                    Text(machine.host)
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                        .truncationMode(.middle)
                }
                .frame(maxWidth: .infinity, alignment: .leading)
            }
            .buttonStyle(.plain)
            .navigationLinkIndicatorVisibility(.hidden)
            if flock?.link != .connecting, let error = flock?.lastError {
                MachineConnectionDetails(message: error)
            }
        }
        .padding(10)
        .glassEffect(.regular, in: .rect(cornerRadius: 16))
    }
}

private struct MachineConnectionDetails: View {
    let message: String
    @State private var copiedAt: Date?

    var body: some View {
        HStack(spacing: 4) {
            Text(message)
                .font(.footnote)
                .foregroundStyle(.orange)
                .multilineTextAlignment(.center)
                .frame(maxWidth: .infinity)
            Button {
                UIPasteboard.general.string = message
                copiedAt = .now
            } label: {
                CopyIcon(label: "Copy connection error", copiedAt: $copiedAt)
                    .padding(6)
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
        }
        .onChange(of: message) { _, _ in copiedAt = nil }
    }
}

/// The clipboard icon, a check for 3 seconds after a copy.
struct CopyIcon: View {
    let label: String
    @Binding var copiedAt: Date?
    @Environment(\.appAccent) private var accent

    var body: some View {
        Label(copiedAt == nil ? label : "Copied",
              systemImage: copiedAt == nil ? "doc.on.clipboard" : AgentActivityStatus(.done).symbol)
            .labelStyle(.iconOnly)
            .font(.footnote.weight(.semibold))
            .foregroundStyle(copiedAt == nil ? accent : AgentState.done.color)
            .frame(minWidth: 28, minHeight: 28)
            .darkGlass(in: .circle, interactive: true)
            .task(id: copiedAt) {
                guard copiedAt != nil else { return }
                try? await Task.sleep(for: .seconds(3))
                guard !Task.isCancelled else { return }
                copiedAt = nil
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
    @State private var renaming: Machine?

    private var typed: String { base.trimmingCharacters(in: .whitespacesAndNewlines) }
    private var current: Machine { app.machines.first { $0.id == machine.id } ?? machine }

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
        .navigationTitle(current.label)
        .toolbar {
            Button("Rename", systemImage: "pencil") { renaming = current }
        }
        .renameMachine($renaming, app: app)
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

private struct RenameMachine: ViewModifier {
    @Binding var machine: Machine?
    let app: AppModel
    @State private var name = ""
    @State private var error: String?

    func body(content: Content) -> some View {
        content
            .alert("Rename machine", isPresented: Binding(get: { machine != nil }, set: { if !$0 { machine = nil } }), presenting: machine) { machine in
                TextField(machine.defaultLabel ?? machine.label, text: $name)
                    .textInputAutocapitalization(.words)
                Button("Cancel", role: .cancel) {}
                Button("Save") { rename(machine, to: name) }
                if let defaultLabel = machine.defaultLabel {
                    Button("Reset to \(defaultLabel)") { rename(machine, to: nil) }
                }
            } message: { _ in
                Text("Shown only on this phone.")
            }
            .alert("Could not rename", isPresented: Binding(get: { error != nil }, set: { if !$0 { error = nil } })) {
                Button("OK") { error = nil }
            } message: {
                Text(error ?? "")
            }
            .onChange(of: machine?.id) { name = machine?.label ?? "" }
    }

    private func rename(_ machine: Machine, to name: String?) {
        do {
            try app.renameMachine(machine, to: name)
        } catch {
            self.error = describe(error)
        }
    }
}

extension View {
    func renameMachine(_ machine: Binding<Machine?>, app: AppModel) -> some View {
        modifier(RenameMachine(machine: machine, app: app))
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
