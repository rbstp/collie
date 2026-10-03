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
        return true
    }

    func application(_ application: UIApplication, didRegisterForRemoteNotificationsWithDeviceToken deviceToken: Data) {
        app.registerPush(token: deviceToken)
    }

    func application(_ application: UIApplication, didFailToRegisterForRemoteNotificationsWithError error: any Error) {
        app.pushRegistrationFailed(error)
    }

    nonisolated func userNotificationCenter(
        _ center: UNUserNotificationCenter,
        willPresent notification: UNNotification
    ) async -> UNNotificationPresentationOptions {
        [.banner, .list, .sound]
    }

    nonisolated func userNotificationCenter(
        _ center: UNUserNotificationCenter,
        didReceive response: UNNotificationResponse
    ) async {
        let content = response.notification.request.content
        let action = NotificationResponse(actionIdentifier: response.actionIdentifier, userInfo: content.userInfo)
        let agent = content.title
        let thread = content.threadIdentifier
        await handle(action, agent: agent, thread: thread)
    }

    private func handle(_ action: NotificationResponse, agent: String, thread: String) async {
        switch action {
        case .decide(let link, let decision):
            await app.decideFromNotification(link, decision, agent: agent, thread: thread)
        case .open(let link):
            app.open(link)
        case .ignore:
            break
        }
    }
}
