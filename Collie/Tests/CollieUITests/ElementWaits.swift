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
