import SwiftUI

@main
struct CollieApp: App {
    @UIApplicationDelegateAdaptor(AppDelegate.self) private var delegate
    @Environment(\.scenePhase) private var scenePhase

    var body: some Scene {
        WindowGroup {
            #if DEBUG
            if let demo = AgentDemo(arguments: CommandLine.arguments) {
                demo
            } else if let demo = ApprovalsDemo(arguments: CommandLine.arguments) {
                demo
            } else {
                RootView(app: delegate.app)
                    .task { await delegate.app.launch() }
                    .onOpenURL { delegate.app.open($0) }
            }
            #else
            RootView(app: delegate.app)
                .task { await delegate.app.launch() }
                .onOpenURL { delegate.app.open($0) }
            #endif
        }
        .onChange(of: scenePhase) { _, phase in
            #if DEBUG
            // The demos run without the app model; its foreground work would end their Live Activity.
            if ["--terminal-demo", "--approvals-demo"].contains(where: CommandLine.arguments.contains) { return }
            #endif
            delegate.app.scenePhaseChanged(to: phase)
        }
    }
}

struct RootView: View {
    @Bindable var app: AppModel

    var body: some View {
        if let error = app.startupError {
            ContentUnavailableView("collie could not start", systemImage: "exclamationmark.triangle", description: Text(error))
        } else if app.showsMain {
            TabView(selection: $app.tab) {
                Tab("Agents", systemImage: "square.grid.2x2", value: AppTab.agents) {
                    FlockScreen(
                        core: app.core, machines: app.machines, approvals: app.approvals, follows: app.follows,
                        opening: $app.openingAgent, tailnetStarting: !app.isRunning
                    ) { app.viewingAgent = $0 }
                }
                Tab("Approvals", systemImage: "checkmark.shield", value: AppTab.approvals) {
                    ApprovalsScreen(model: app.approvals)
                }
                .badge(app.approvals.items.count)
                Tab("Machines", systemImage: "desktopcomputer", value: AppTab.machines) { MachinesView(app: app) }
                Tab("Settings", systemImage: "gearshape", value: AppTab.settings) { SettingsView(app: app) }
            }
            .task { await app.enableNotifications() }
            .task {
                while !Task.isCancelled {
                    app.approvals.poll()
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
            OnboardingView(app: app)
        }
    }
}
