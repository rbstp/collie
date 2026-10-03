import Foundation
import UserNotifications

final class NotificationService: UNNotificationServiceExtension {
    override func didReceive(
        _ request: UNNotificationRequest,
        withContentHandler contentHandler: @escaping (UNNotificationContent) -> Void
    ) {
        guard let content = request.content.mutableCopy() as? UNMutableNotificationContent else {
            contentHandler(request.content)
            return
        }
        let hint = AppGroup.container.flatMap { try? Data(contentsOf: $0.appending(path: MacReachability.fileName)) }
        content.body = PushBody.rewrite(content.body, nodeId: content.userInfo["node_id"] as? String, reachability: hint)
        contentHandler(content)
    }
}
