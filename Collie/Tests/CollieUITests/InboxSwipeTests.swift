import XCTest

@MainActor
final class InboxSwipeTests: XCTestCase {
    func testSwipeRightOffersSnoozeExceptOnBlockedRows() throws {
        continueAfterFailure = false
        let app = try demoApp("attachment-demo")
        app.launch()
        defer { app.terminate() }
        let layout = app.buttons["Layout"]
        XCTAssertTrue(layout.appears(within: 10), app.debugDescription)
        app.choose(app.buttons["Inbox"]) { layout.tap() }
        let row = app.staticTexts["Dark mode follows the system setting on every page."]
        XCTAssertTrue(row.appears(within: 5), app.debugDescription)
        let snooze = app.buttons["Snooze"]
        for _ in 0..<2 {
            // A fixed distance: past about half the row, the drag is a full swipe that snoozes without showing the button.
            let start = row.coordinate(withNormalizedOffset: CGVector(dx: 0.1, dy: 0.5))
            start.press(forDuration: 0.1, thenDragTo: start.withOffset(CGVector(dx: 180, dy: 0)), withVelocity: .slow, thenHoldForDuration: 0)
            XCTAssertTrue(snooze.appears(within: 5), app.debugDescription)
            XCTAssertTrue(app.segmentedControls["View"].buttons["Agents"].isSelected, app.debugDescription)
            snooze.tap()
            let hour = app.buttons["1 hour"]
            XCTAssertTrue(hour.appears(within: 5), app.debugDescription)
            app.otherElements["PopoverDismissRegion"].firstMatch.tap()
            XCTAssertTrue(hour.disappears(within: 5), app.debugDescription)
        }
        let blocked = app.staticTexts["I need to run cargo test to check the new cases."]
        XCTAssertTrue(blocked.appears(within: 5), app.debugDescription)
        blocked.swipeRight()
        XCTAssertFalse(snooze.exists, app.debugDescription)
    }

    func testSearchOpensInTheFilterRowAndKeepsTheLastRowAboveTheKeyboard() throws {
        continueAfterFailure = false
        let app = try demoApp("attachment-demo")
        app.launch()
        defer { app.terminate() }
        let layout = app.buttons["Layout"]
        XCTAssertTrue(layout.appears(within: 10), app.debugDescription)
        app.choose(app.buttons["Inbox"]) { layout.tap() }
        let filter = app.segmentedControls["Filter"].buttons.firstMatch
        XCTAssertNotNil(filter.settledFrame(), app.debugDescription)
        let row = filter.frame
        app.buttons["Search"].tap()
        let search = app.textFields.matching(NSPredicate(format: "placeholderValue == %@", "Search agents")).firstMatch
        let keyboard = app.keyboards.firstMatch
        XCTAssertTrue(keyboard.appears(within: 10), app.debugDescription)
        XCTAssertNotNil(search.settledFrame(), app.debugDescription)
        let before = search.frame
        XCTAssertLessThan(abs(before.midY - row.midY), 4, app.debugDescription)
        XCTAssertFalse(filter.exists, app.debugDescription)
        for _ in 0..<3 { app.swipeUp() }
        let last = app.staticTexts["The rate limiter is merged."]
        XCTAssertNotNil(last.settledFrame(), app.debugDescription)
        XCTAssertEqual(search.frame, before, app.debugDescription)
        XCTAssertTrue(last.isHittable, app.debugDescription)
        XCTAssertLessThanOrEqual(last.frame.maxY, keyboard.frame.minY, app.debugDescription)
        app.buttons["Close search"].tap()
        XCTAssertTrue(filter.appears(within: 5), app.debugDescription)
    }
}
