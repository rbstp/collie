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
        // The sender's subtitle would show between the authenticated title and body.
        content.subtitle = ""
        content.body = content.body == "An agent finished" ? "An agent finished" : "An agent needs attention"
        content.categoryIdentifier = ""
        content.userInfo[PushDisplay.shownKey] = nil
        if let nodeId, let key = NotificationKey.load(nodeId: nodeId),
            let display = PushDisplay.open(userInfo: content.userInfo, key: key) {
            content.title = display.shownTitle
            content.body = display.shownBody
            content.userInfo["terminal_id"] = display.terminalId
            content.threadIdentifier = display.terminalId
            content.categoryIdentifier = display.category
            if let enc = content.userInfo["enc"] as? String {
                content.userInfo[PushDisplay.shownKey] = PushDisplay.shownMark(enc: enc, key: key)
            }
        }
        let hint = AppGroup.container.flatMap { try? Data(contentsOf: $0.appending(path: MacReachability.fileName)) }
        content.body = PushBody.rewrite(content.body, nodeId: nodeId, reachability: hint)
        contentHandler(content)
    }
}
