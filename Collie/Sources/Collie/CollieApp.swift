import SwiftUI

@main
struct CollieApp: App {
    @State private var app = AppModel()
    @Environment(\.scenePhase) private var scenePhase

    var body: some Scene {
        WindowGroup {
            #if DEBUG
            if let demo = AgentDemo(arguments: CommandLine.arguments) {
                demo
            } else {
                RootView(app: app)
                    .task { await app.launch() }
            }
            #else
            RootView(app: app)
                .task { await app.launch() }
            #endif
        }
        .onChange(of: scenePhase) { _, phase in
            app.scenePhaseChanged(to: phase)
        }
    }
}

struct RootView: View {
    let app: AppModel

    var body: some View {
        if let error = app.startupError {
            ContentUnavailableView("collie could not start", systemImage: "exclamationmark.triangle", description: Text(error))
        } else if app.isRunning {
            TabView {
                Tab("Agents", systemImage: "square.grid.2x2") { FlockScreen(app: app) }
                Tab("Machines", systemImage: "desktopcomputer") { MachinesView(app: app) }
                Tab("Settings", systemImage: "gearshape") { SettingsView(app: app) }
            }
            .task {
                while !Task.isCancelled && app.isRunning {
                    try? await Task.sleep(for: .seconds(5))
                    await app.refreshNode()
                }
            }
        } else {
            OnboardingView(app: app)
        }
    }
}
