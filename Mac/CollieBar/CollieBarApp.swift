import AppKit
import SwiftUI

@main
struct CollieBarApp: App {
    @State private var daemon = Daemon()

    var body: some Scene {
        MenuBarExtra {
            MenuContent(daemon: daemon)
        } label: {
            Image(nsImage: MenuIcon.image(running: daemon.running, pending: daemon.pendingApprovals > 0))
                .accessibilityLabel(MenuIcon.label(daemon.state))
        }
        .menuBarExtraStyle(.menu)

        Window("Pair a Phone", id: "pair") {
            PairingView(daemon: daemon)
        }
        .windowResizability(.contentSize)
        .restorationBehavior(.disabled)
        .defaultLaunchBehavior(.suppressed)
    }
}

struct MenuContent: View {
    let daemon: Daemon
    @Environment(\.openWindow) private var openWindow

    var body: some View {
        Text(verbatim: status)
        if daemon.state == .outdated {
            Text("Update collied (just collied-install)")
        }
        if daemon.pendingApprovals > 0 {
            Text(verbatim: MenuIcon.pendingText(daemon.pendingApprovals))
        }
        if let error = daemon.lastError {
            Text(verbatim: error)
        }
        Button(daemon.running ? "Turn Off" : "Turn On") { daemon.toggle() }
            .disabled(daemon.busy || daemon.starting)
        Divider()
        Button("Pair a Phone…") {
            NSApp.activate()
            openWindow(id: "pair")
        }
        .disabled(!daemon.running)
        Menu("Paired Phones") {
            if !daemon.running {
                Text("collied is off")
            } else if daemon.peers.isEmpty {
                Text("None")
            } else {
                ForEach(daemon.peers, id: \.stableId) { peer in
                    Text(verbatim: "\(printable(peer.label)) (\(printable(peer.login)))")
                }
            }
        }
        Divider()
        Button("Open Audit Log") { daemon.openAuditLog() }
            .disabled(!daemon.auditLogExists)
        Toggle("Open at Login", isOn: Binding(get: { daemon.openAtLogin }, set: { daemon.setOpenAtLogin($0) }))
        Divider()
        Button("Quit (stops collied)") { daemon.quit() }
            .keyboardShortcut("q")
    }

    private var status: String {
        if daemon.starting { return "collied: Starting…" }
        return daemon.running ? "collied: Running" : "collied: Off"
    }
}
