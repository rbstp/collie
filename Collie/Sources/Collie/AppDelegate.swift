import CollieCore
import UIKit
import UserNotifications

/// Owns the AppModel so a notification action can be handled when iOS launches the app in
/// the background with no scene.
final class AppDelegate: NSObject, UIApplicationDelegate, UNUserNotificationCenterDelegate {
    let app = AppModel()

    func application(
        _ application: UIApplication,
        didFinishLaunchingWithOptions launchOptions: [UIApplication.LaunchOptionsKey: Any]? = nil
    ) -> Bool {
        let center = UNUserNotificationCenter.current()
        center.setNotificationCategories(ApprovalNotification.categories)
        center.delegate = self
        DecideApprovalIntent.decide = { [app] link, decision in await app.decideFromActivity(link, decision) }
        app.watch.activate(decide: { [app] in await app.decideFromWatch($0) }, refresh: { [app] in await app.refreshForWatch() })
        return true
    }

    func application(_ application: UIApplication, didRegisterForRemoteNotificationsWithDeviceToken deviceToken: Data) {
        app.registerPush(token: deviceToken)
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
        let action = NotificationResponse(actionIdentifier: response.actionIdentifier, userInfo: content.userInfo)
        let agent = content.title
        let thread = content.threadIdentifier
        Task { @MainActor in
            await self.handle(action, agent: agent, thread: thread)
            completionHandler()
        }
    }

    private func handle(_ action: NotificationResponse, agent: String, thread: String) async {
        switch action {
        case .decide(let link, let decision):
            await app.decideFromNotification(link, decision, agent: agent, thread: thread)
        case .open(let link):
            app.open(link)
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
