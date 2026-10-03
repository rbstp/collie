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
        let nodeId = content.userInfo["node_id"] as? String
        if let nodeId, let enc = content.userInfo["enc"] as? String, let approvalId = content.userInfo["approval_id"] as? String,
            let key = NotificationKey.load(nodeId: nodeId),
            let context = PushContext.open(enc, approvalId: approvalId, key: key)
        {
            content.body = context
        }
        let hint = AppGroup.container.flatMap { try? Data(contentsOf: $0.appending(path: MacReachability.fileName)) }
        content.body = PushBody.rewrite(content.body, nodeId: nodeId, reachability: hint)
        contentHandler(content)
    }
}
