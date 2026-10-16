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
        content.title = "Collie"
        content.body = content.body == "An agent finished" ? "An agent finished" : "An agent needs attention"
        content.categoryIdentifier = ""
        if let nodeId, let key = NotificationKey.load(nodeId: nodeId),
            let display = PushDisplay.open(userInfo: content.userInfo, key: key) {
            content.title = PushDisplay.clean(display.title, maxLength: 64)
            content.body = PushDisplay.clean(display.body, maxLength: 600)
            content.userInfo["terminal_id"] = display.terminalId
            content.threadIdentifier = display.terminalId
            if display.offersApproval, display.approvalId != nil { content.categoryIdentifier = "APPROVAL" }
        }
        let hint = AppGroup.container.flatMap { try? Data(contentsOf: $0.appending(path: MacReachability.fileName)) }
        content.body = PushBody.rewrite(content.body, nodeId: nodeId, reachability: hint)
        contentHandler(content)
    }
}
