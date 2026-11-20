import CollieCore
import UIKit
import UserNotifications

/// Owns the AppModel so a notification action can be handled when iOS launches the app in
/// the background with no scene.
final class AppDelegate: NSObject, UIApplicationDelegate, UNUserNotificationCenterDelegate {
    private(set) var app = AppModel()

    func application(
        _ application: UIApplication,
        didFinishLaunchingWithOptions launchOptions: [UIApplication.LaunchOptionsKey: Any]? = nil
    ) -> Bool {
        let center = UNUserNotificationCenter.current()
        center.setNotificationCategories(ApprovalNotification.categories)
        center.delegate = self
        wire()
        NotificationCenter.default.addObserver(forName: UIWindow.didBecomeKeyNotification, object: nil, queue: .main) { note in
            guard let window = note.object as? UIWindow else { return }
            MainActor.assumeIsolated { KeyboardSwipe.shared.install(on: window) }
        }
        WindowCover.observe()
        FileViewerModel.purgeCopies()
        return true
    }

    private func wire() {
        DecideApprovalIntent.decide = { [app] link, decision in await app.decideFromActivity(link, decision) }
        app.watch.activate(decide: { [app] in await app.decideFromWatch($0) }, refresh: { [app] in await app.refreshForWatch() })
    }

    /// Only from the owner's confirmation on the startup screen. A new model then starts as at
    /// launch and makes the keys a reset deleted; the sign-in, the settings and the machines stay.
    func resetKey() {
        EnclaveBlob.reset { _ = try Identity.load(unlocked: UIApplication.shared.isProtectedDataAvailable) }
        app = AppModel()
        wire()
    }

    func application(_ application: UIApplication, didRegisterForRemoteNotificationsWithDeviceToken deviceToken: Data) {
        app.didRegisterPush(token: deviceToken)
    }

    func application(_ application: UIApplication, didFailToRegisterForRemoteNotificationsWithError error: any Error) {
        app.pushRegistrationFailed(error)
    }

    func application(
        _ application: UIApplication,
        didReceiveRemoteNotification userInfo: [AnyHashable: Any],
        fetchCompletionHandler completionHandler: @escaping (UIBackgroundFetchResult) -> Void
    ) {
        guard let link = ApprovalClear.link(userInfo) else {
            completionHandler(.noData)
            return
        }
        Task {
            await ApprovalClear.remove(link)
            completionHandler(.noData)
        }
    }

    // The completion handlers must run on the main thread: with the async variants the
    // system calls them from the cooperative pool and UIKit aborts (state restoration assert).
    nonisolated func userNotificationCenter(
        _ center: UNUserNotificationCenter,
        willPresent notification: UNNotification,
        withCompletionHandler completionHandler: @escaping @Sendable (UNNotificationPresentationOptions) -> Void
    ) {
        let response = NotificationResponse(
            actionIdentifier: UNNotificationDefaultActionIdentifier, userInfo: notification.request.content.userInfo
        )
        Task { @MainActor in
            if case .openAgent(let nodeId, let terminalId) = response, self.app.isViewing(nodeId: nodeId, terminalId: terminalId) {
                completionHandler([])
            } else {
                completionHandler([.banner, .list, .sound])
            }
        }
    }

    nonisolated func userNotificationCenter(
        _ center: UNUserNotificationCenter,
        didReceive response: UNNotificationResponse,
        withCompletionHandler completionHandler: @escaping @Sendable () -> Void
    ) {
        let content = response.notification.request.content
        let action = NotificationResponse(
            actionIdentifier: response.actionIdentifier, userInfo: content.userInfo,
            title: content.title, subtitle: content.subtitle, body: content.body, category: content.categoryIdentifier
        )
        let display = (content.userInfo["node_id"] as? String)
            .flatMap { NotificationKey.load(nodeId: $0) }
            .flatMap { PushDisplay.open(userInfo: content.userInfo, key: $0) }
        let agent = display?.shownTitle ?? "agent"
        let thread = display?.terminalId ?? content.threadIdentifier
        let refused = ApprovalNotification.decision(forAction: response.actionIdentifier) != nil
        Task { @MainActor in
            await self.handle(action, agent: agent, thread: thread)
            // Approve and Deny run in the background: without this, a refused one would show nothing at all.
            if refused, case .open(let link) = action { await self.app.post(.refused(agent: agent), for: link, thread: thread) }
            completionHandler()
        }
    }

    private func handle(_ action: NotificationResponse, agent: String, thread: String) async {
        switch action {
        case .decide(let link, let decision):
            await app.decideFromNotification(link, decision, agent: agent, thread: thread)
        case .open(let link):
            app.open(link, terminalId: thread)
        case .openAgent(let nodeId, let terminalId):
            if let machine = app.machines.first(where: { $0.nodeId == nodeId }),
                let link = AgentLink(machineId: machine.id, terminalId: terminalId)
            {
                app.open(link.url)
            }
        case .ignore:
            break
        }
    }
}

/// Keeps a terminal, a chat or an open file out of the app switcher snapshot, which iOS stores on disk.
final class WindowCover: UIView {
    // iOS takes the snapshot once these observers have run, so the cover goes into the window here, synchronously:
    // an overlay SwiftUI draws on a scenePhase change can come too late. Not on deactivation: the Face ID sheet
    // makes the app inactive.
    @MainActor static func observe(_ center: NotificationCenter = .default) {
        center.addObserver(forName: UIScene.didEnterBackgroundNotification, object: nil, queue: nil) { note in
            guard let scene = note.object as? UIWindowScene else { return }
            MainActor.assumeIsolated {
                for window in scene.windows where !window.subviews.contains(where: { $0 is WindowCover }) {
                    let cover = WindowCover(frame: window.bounds)
                    cover.backgroundColor = .systemBackground
                    cover.autoresizingMask = [.flexibleWidth, .flexibleHeight]
                    // Above a view the window gets later, as UIKit adds one for a sheet or an alert presented in the background.
                    cover.layer.zPosition = .greatestFiniteMagnitude
                    window.addSubview(cover)
                }
            }
        }
        center.addObserver(forName: UIScene.willEnterForegroundNotification, object: nil, queue: nil) { note in
            guard let scene = note.object as? UIWindowScene else { return }
            MainActor.assumeIsolated {
                for window in scene.windows {
                    window.subviews.filter { $0 is WindowCover }.forEach { $0.removeFromSuperview() }
                }
            }
        }
    }
}
