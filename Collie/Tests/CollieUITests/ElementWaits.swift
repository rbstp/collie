import XCTest

@MainActor
extension XCUIElement {
    // waitForExistence first polls after one second, even when the element is already there.
    func appears(within timeout: TimeInterval) -> Bool {
        exists || waitForExistence(timeout: timeout)
    }

    func disappears(within timeout: TimeInterval) -> Bool {
        !exists || waitForNonExistence(timeout: timeout)
    }
}

@MainActor
extension XCUIApplication {
    // While a system menu is open, UIKit can keep a finished pacing animation on a hidden view, and XCTest's wait for
    // animations to end then runs out its 60 s after the event that opens the menu and again before the item tap.
    // XCTest's private interaction options skip those two waits: 1 skips the wait before an event, 2 skips both.
    func choose(_ item: XCUIElement, opening open: () -> Void) {
        withInteractionOptions(2, open)
        XCTAssertTrue(item.wait(for: \.isHittable, toEqual: true, timeout: 5), debugDescription)
        withInteractionOptions(1) { item.tap() }
    }

    private func withInteractionOptions(_ options: UInt32, _ body: () -> Void) {
        let selector = NSSelectorFromString("_performWithInteractionOptions:block:")
        guard responds(to: selector) else { return body() }
        typealias Perform = @convention(c) (AnyObject, Selector, UInt32, @convention(block) () -> Void) -> Void
        withoutActuallyEscaping(body) { body in
            unsafeBitCast(method(for: selector), to: Perform.self)(self, selector, options, body)
        }
    }
}
