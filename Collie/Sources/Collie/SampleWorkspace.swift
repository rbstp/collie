import SwiftUI

struct SampleWorkspace: View {
    @Environment(\.dismiss) private var dismiss
    @State private var generation = UUID()

    var body: some View {
        SampleSession(exit: { dismiss() }, reset: { generation = UUID() })
            .id(generation)
    }
}

private struct SampleSession: View {
    let exit: () -> Void
    let reset: () -> Void
    @State private var core: DemoAgentCore
    @State private var approvals: ApprovalsModel
    @State private var opening: AgentRoute?

    init(exit: @escaping () -> Void, reset: @escaping () -> Void) {
        self.exit = exit
        self.reset = reset
        let core = DemoAgentCore()
        _core = State(initialValue: core)
        _approvals = State(initialValue: ApprovalsModel(core: core, auth: SampleAuthentication(), dismissResolved: { _, _ in }))
    }

    var body: some View {
        tabs
            .tabViewBottomAccessory {
                HStack {
                    VStack(alignment: .leading) {
                        Text("Sample workspace").font(.headline)
                        Text("Simulated actions only").font(.caption).foregroundStyle(.secondary)
                    }
                    Spacer()
                    Button("Exit", action: exit)
                }
                .padding(.horizontal)
            }
            .task {
                while !Task.isCancelled {
                    approvals.poll()
                    try? await Task.sleep(for: .milliseconds(250))
                }
            }
    }

    private var tabs: some View {
        TabView {
            Tab("Agents", systemImage: "square.grid.2x2") {
                FlockScreen(core: core, machines: [DemoAgentCore.machine], approvals: approvals, follows: nil, opening: $opening, persistsState: false)
            }
            Tab("Machines", systemImage: "desktopcomputer") {
                NavigationStack {
                    List {
                        MachineRow(machine: DemoAgentCore.machine, flock: core.cachedFlock(machineId: DemoAgentCore.machine.id)) {
                            Text("Sample machine").font(.headline)
                            Text("This machine and its agents are synthetic. No connection is made.")
                        }
                    }
                    .navigationTitle("Machines")
                }
            }
            Tab("Settings", systemImage: "gearshape") {
                NavigationStack {
                    Form {
                        Section {
                            Text("Explore Chat, Terminal, Usage and approvals. Every action is simulated on this device. No command runs on a real machine.")
                        }
                        Section {
                            Button("Reset sample workspace", action: reset)
                            Button("Exit sample workspace", action: exit)
                        } footer: {
                            Text("Your real accounts and paired machines are kept separately.")
                        }
                    }
                    .navigationTitle("Sample settings")
                }
            }
        }
    }

    // This authenticator is private to the synthetic core above, never the paired-machine path.
    private struct SampleAuthentication: Authenticator {
        func authenticate(reason: String) async -> Bool { true }
    }
}
