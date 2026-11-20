import SwiftUI

@main
struct CollieApp: App {
    @UIApplicationDelegateAdaptor(AppDelegate.self) private var delegate
    @Environment(\.scenePhase) private var scenePhase
    @State private var restarts = 0

    var body: some Scene {
        WindowGroup {
            Group {
                #if DEBUG
                if let demo = AgentDemo(arguments: CommandLine.arguments) {
                    demo
                } else {
                    RootView(app: delegate.app, resetKey: resetKey)
                        .task { await delegate.app.launch() }
                        .onOpenURL { delegate.app.open($0) }
                        .id(restarts)
                }
                #else
                RootView(app: delegate.app, resetKey: resetKey)
                    .task { await delegate.app.launch() }
                    .onOpenURL { delegate.app.open($0) }
                    .id(restarts)
                #endif
            }
            .dynamicTypeSize(...DynamicTypeSize.accessibility3)
        }
        .onChange(of: scenePhase) { _, phase in
            #if DEBUG
            // The demos run without the app model; its foreground work would end their Live Activity.
            if CommandLine.arguments.contains("--terminal-demo") { return }
            #endif
            delegate.app.scenePhaseChanged(to: phase)
        }
    }

    /// The delegate's model is a new one afterwards: the root is built again around it, and it
    /// is told the scene's phase as a launch tells it.
    private func resetKey() {
        delegate.resetKey()
        delegate.app.scenePhaseChanged(to: scenePhase)
        restarts += 1
    }
}

struct RootView: View {
    @Bindable var app: AppModel
    let resetKey: () -> Void
    @State private var agentsReselected = 0
    @State private var sample = false
    @State private var resetting = false

    var body: some View {
        content
            .fullScreenCover(isPresented: $sample) { SampleWorkspace() }
            .tint(app.accent.color)
            .accentColor(app.accent.color)
            .environment(\.appAccent, app.accent.color)
            .environment(\.appGlass, app.glass)
            .environment(\.runtimeMarks, app.runtimeMarks)
            .environment(\.neutralAccent, app.accent == .neutral)
            .toggleStyle(AccentSwitch(tint: app.accent.switchTint))
            .alert("Approval unavailable", isPresented: Binding(
                get: { app.approvalOpenError != nil },
                set: { if !$0 { app.approvalOpenError = nil } }
            )) {
                Button("OK") { app.approvalOpenError = nil }
            } message: {
                Text(app.approvalOpenError ?? "")
            }
    }

    @ViewBuilder
    private var content: some View {
        if let error = app.startupError {
            ContentUnavailableView {
                Label("collie could not start", systemImage: "exclamationmark.triangle")
            } description: {
                Text(error)
            } actions: {
                Button("Explore sample workspace", systemImage: "play.rectangle") { sample = true }
                    .appGlassButton()
                if app.keyKept {
                    Button("Reset this phone's key", systemImage: "key.slash", role: .destructive) { resetting = true }
                        .tint(.red)
                        .appGlassButton()
                        .confirmationDialog("Reset this phone's key?", isPresented: $resetting, titleVisibility: .visible) {
                            Button("Reset this phone's key", role: .destructive, action: resetKey)
                            Button("Cancel", role: .cancel) {}
                        } message: {
                            Text("This phone gets a new key. On each machine, run collied peers revoke for this phone, then pair again.")
                        }
                }
            }
        } else if app.showsMain {
            TabView(
                selection: Binding {
                    app.tab
                } set: { tab in
                    app.cancelApprovalOpening()
                    if tab == .agents, app.tab == .agents { agentsReselected += 1 }
                    app.tab = tab
                }
            ) {
                Tab("Agents", systemImage: "sparkles.rectangle.stack", value: AppTab.agents) {
                    FlockScreen(
                        core: app.core, machines: app.machines, approvals: app.approvals, follows: app.follows,
                        opening: $app.openingAgent, tailnetStarting: !app.isRunning, reselected: agentsReselected,
                        navigationChanged: app.cancelApprovalOpening
                    ) { route in
                        app.viewingAgent = route
                    }
                    .safeAreaInset(edge: .top) {
                        if let loading = app.approvals.loading {
                            VStack(alignment: .leading, spacing: 2) {
                                ProgressView("Opening \(loading.machine.label)").font(.footnote)
                                if let phase = loading.phase {
                                    Text(phase.label).font(.caption).foregroundStyle(.secondary)
                                }
                            }
                            .padding(8)
                            .appGlass(in: .rect(cornerRadius: 12))
                        }
                    }
                }
                Tab("Machines", systemImage: "desktopcomputer", value: AppTab.machines) { MachinesView(app: app) }
                Tab("Settings", systemImage: "gearshape", value: AppTab.settings) { SettingsView(app: app, openSample: { sample = true }) }
            }
            .task { await app.enableNotifications() }
            .task {
                while !Task.isCancelled {
                    app.approvals.poll()
                    app.refreshRelayRegistration()
                    app.publishWatchState()
                    try? await Task.sleep(for: .seconds(1))
                }
            }
            .task(id: app.isRunning) {
                while !Task.isCancelled && app.showsMain {
                    try? await Task.sleep(for: .seconds(app.isRunning ? 30 : 1))
                    await app.refreshNode()
                }
            }

        } else {
            OnboardingView(app: app, openSample: { sample = true })
        }
    }
}
