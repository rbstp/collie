import SwiftUI

@main
struct CollieWatchApp: App {
    var body: some Scene {
        WindowGroup {
            ContentUnavailableView("Open collie on the iPhone", systemImage: "iphone")
        }
    }
}
