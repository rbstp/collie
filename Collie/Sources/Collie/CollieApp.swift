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
            }
            #else
            RootView(app: delegate.app)
                .task { await delegate.app.launch() }
            #endif
        }
        .onChange(of: scenePhase) { _, phase in
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
                    FlockScreen(core: app.core, machines: app.machines, approvals: app.approvals, tailnetStarting: !app.isRunning)
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
                    try? await Task.sleep(for: .seconds(1))
                }
            }
            .task {
                while !Task.isCancelled && app.showsMain {
                    try? await Task.sleep(for: .seconds(app.isRunning ? 5 : 1))
                    await app.refreshNode()
                }
            }

        } else {
            OnboardingView(app: app)
        }
    }
}
