import XCTest

@MainActor
extension XCUIElement {
    // Not waitForExistence: after every miss it snapshots the whole app for a debug description, and on a loaded runner
    // those snapshots took seconds each and ended in "Timed out while evaluating UI query".
    func appears(within timeout: TimeInterval) -> Bool {
        poll(timeout) { exists }
    }

    func disappears(within timeout: TimeInterval) -> Bool {
        poll(timeout) { !exists }
    }

    private func poll(_ timeout: TimeInterval, _ condition: () -> Bool) -> Bool {
        let deadline = Date.now.addingTimeInterval(timeout)
        while !condition() {
            if Date.now >= deadline { return false }
            usleep(250_000)
        }
        return true
    }
}

@MainActor
extension XCUIApplication {
    /// Types into a focused field, clearing and typing again when the simulator drops keystrokes, and returns the field by
    /// its value, which a vertical prompt can surface as a text view.
    func type(_ text: String, into field: XCUIElement) -> XCUIElement {
        // The main window only: an app-wide value query also walks the keyboard's windows, and it timed out on a loaded
        // runner while the keyboard was resyncing with the field.
        let typed = windows.firstMatch.descendants(matching: .any).matching(NSPredicate(format: "value == %@", text)).firstMatch
        let clear = buttons["Clear text"]
        if clear.exists { clear.tap() }
        field.typeText(text)
        for _ in 0..<3 where !typed.appears(within: 10) {
            if clear.exists { clear.tap() }
            field.typeText(text)
        }
        XCTAssertTrue(typed.appears(within: 10), debugDescription)
        return typed
    }

    // While a system menu is open, UIKit can keep a finished pacing animation on a hidden view, and XCTest's wait for
    // animations to end then runs out its 60 s after the event that opens the menu and again before the item tap.
    // XCTest's private interaction options skip those two waits: 1 skips the wait before an event, 2 skips both.
    func choose(_ item: XCUIElement, opening open: () -> Void) {
        withInteractionOptions(2, open)
        // The item exists before the menu has laid it out, and asking a frameless item for isHittable records a failure.
        let deadline = Date.now.addingTimeInterval(10)
        while Date.now < deadline, !(item.exists && !item.frame.isEmpty && frame.contains(item.frame)) {
            usleep(200_000)
        }
        XCTAssertTrue(item.wait(for: \.isHittable, toEqual: true, timeout: 5), debugDescription)
        withInteractionOptions(1) { item.tap() }
    }

    func withInteractionOptions(_ options: UInt32, _ body: () -> Void) {
        let selector = NSSelectorFromString("_performWithInteractionOptions:block:")
        guard responds(to: selector) else { return body() }
        typealias Perform = @convention(c) (AnyObject, Selector, UInt32, @convention(block) () -> Void) -> Void
        withoutActuallyEscaping(body) { body in
            unsafeBitCast(method(for: selector), to: Perform.self)(self, selector, options, body)
        }
    }
}
