import XCTest

@MainActor
final class InboxSwipeTests: XCTestCase {
    func testSwipeRightOffersSnoozeExceptOnBlockedRows() throws {
        continueAfterFailure = false
        let app = XCUIApplication()
        let fixture = try XCTUnwrap(Bundle(for: Self.self).url(forResource: "attachment-demo", withExtension: "txt"))
        app.launchArguments = ["-AppleLanguages", "(en)", "-AppleLocale", "en_US", "--terminal-demo", fixture.path]
        app.launch()
        defer { app.terminate() }
        let layout = app.buttons["Layout"]
        XCTAssertTrue(layout.appears(within: 10), app.debugDescription)
        app.choose(app.buttons["Inbox"]) { layout.tap() }
        let row = app.staticTexts["Dark mode follows the system setting on every page."]
        XCTAssertTrue(row.appears(within: 5), app.debugDescription)
        let snooze = app.buttons["Snooze"]
        for _ in 0..<2 {
            row.coordinate(withNormalizedOffset: CGVector(dx: 0.1, dy: 0.5))
                .press(forDuration: 0.1, thenDragTo: row.coordinate(withNormalizedOffset: CGVector(dx: 0.9, dy: 0.5)),
                       withVelocity: .slow, thenHoldForDuration: 0)
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

    func testSearchStaysBelowTheFilterAndTheLastRowAboveTheKeyboard() throws {
        continueAfterFailure = false
        let app = XCUIApplication()
        let fixture = try XCTUnwrap(Bundle(for: Self.self).url(forResource: "attachment-demo", withExtension: "txt"))
        app.launchArguments = ["-AppleLanguages", "(en)", "-AppleLocale", "en_US", "--terminal-demo", fixture.path]
        app.launch()
        defer { app.terminate() }
        let layout = app.buttons["Layout"]
        XCTAssertTrue(layout.appears(within: 10), app.debugDescription)
        app.choose(app.buttons["Inbox"]) { layout.tap() }
        let search = app.textFields.matching(NSPredicate(format: "placeholderValue == %@", "Search agents")).firstMatch
        XCTAssertTrue(search.appears(within: 5), app.debugDescription)
        let before = search.frame
        search.tap()
        let keyboard = app.keyboards.firstMatch
        XCTAssertTrue(keyboard.appears(within: 5), app.debugDescription)
        XCTAssertEqual(search.frame, before, app.debugDescription)
        for _ in 0..<3 { app.swipeUp() }
        sleep(1)
        XCTAssertEqual(search.frame, before, app.debugDescription)
        let last = app.staticTexts["The rate limiter is merged."]
        XCTAssertTrue(last.appears(within: 5), app.debugDescription)
        XCTAssertTrue(last.isHittable, app.debugDescription)
        XCTAssertLessThanOrEqual(last.frame.maxY, keyboard.frame.minY, app.debugDescription)
        let all = app.segmentedControls["Filter"].buttons.firstMatch
        XCTAssertGreaterThanOrEqual(search.frame.minY, all.frame.maxY, app.debugDescription)
    }
}
