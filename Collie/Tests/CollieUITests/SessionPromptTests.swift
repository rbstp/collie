import XCTest

@MainActor
final class SessionPromptTests: XCTestCase {
    func testSuggestionFillsThePromptSavedPromptsOpenOverTheKeyboardAndASwipeDownDismissesIt() throws {
        continueAfterFailure = false
        let suggestion = "run the tests"
        let app = try demoApp("landscape-demo")
        app.launchEnvironment["COLLIE_DEMO_SUGGESTION"] = suggestion
        try app.launch(onSession: "term_demo")
        defer { app.terminate() }
        let shown = app.textFields.matching(NSPredicate(format: "placeholderValue == %@", suggestion)).firstMatch
        XCTAssertTrue(shown.appears(within: 10), app.debugDescription)
        let use = app.buttons["Use suggestion"]
        XCTAssertTrue(use.exists, app.debugDescription)
        use.tap()
        // A prompt with text, typed here or synced from the Mac, leaves the suggestion out.
        XCTAssertTrue(use.disappears(within: 5), app.debugDescription)
        let typed = app.textFields.matching(NSPredicate(format: "value == %@", suggestion)).firstMatch
        XCTAssertTrue(typed.appears(within: 5), app.debugDescription)
        XCTAssertTrue(app.buttons["Send"].isEnabled, app.debugDescription)

        // In the terminal the suggestion's Tab shows in the bottom row, unless the strip's own Tab covers it.
        app.buttons["Terminal"].firstMatch.tap()
        let keys = app.buttons["Keys"]
        XCTAssertTrue(keys.appears(within: 5), app.debugDescription)
        let escape = app.buttons["Escape"]
        let accept = app.buttons["Accept suggestion"]
        if !keys.isSelected { keys.tap() }
        XCTAssertTrue(escape.appears(within: 5), app.debugDescription)
        XCTAssertFalse(accept.exists, app.debugDescription)
        keys.tap()
        XCTAssertTrue(escape.disappears(within: 5), app.debugDescription)
        XCTAssertTrue(accept.appears(within: 5), app.debugDescription)
        let returnKey = app.buttons["Return"]
        XCTAssertTrue(accept.isHittable && returnKey.exists, app.debugDescription)
        XCTAssertEqual(accept.frame.midY, returnKey.frame.midY, accuracy: 2, app.debugDescription)
        XCTAssertLessThan(accept.frame.maxX, returnKey.frame.minX, app.debugDescription)
        accept.tap()
        XCTAssertTrue(accept.disappears(within: 5), app.debugDescription)

        app.buttons["Chat"].firstMatch.tap()
        XCTAssertTrue(typed.appears(within: 5), app.debugDescription)
        typed.tap()
        let keyboard = app.keyboards.firstMatch
        XCTAssertTrue(keyboard.appears(within: 10), app.debugDescription)
        app.choose(app.buttons["Saved prompts"].firstMatch) { app.buttons["Attachments and saved prompts"].tap() }
        let saved = app.navigationBars["Saved prompts"]
        XCTAssertTrue(saved.appears(within: 5), app.debugDescription)
        saved.buttons["Cancel"].tap()
        XCTAssertTrue(saved.disappears(within: 5), app.debugDescription)
        XCTAssertTrue(keyboard.appears(within: 10), app.debugDescription)

        XCTAssertNotNil(typed.settledFrame(), app.debugDescription)
        let center = typed.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5))
        center.press(forDuration: 1, thenDragTo: typed.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 1.5)))
        XCTAssertTrue(keyboard.exists, "a drag after a long press is not a swipe")
        typed.tap()
        // XCUIElement.swipeDown travels a fraction of the element, too short on a one-line field for a swipe.
        // Last: after the keyboard leaves a session the app can stop idling for minutes, and XCTest then waits 60 s around every
        // later event (see choose).
        app.withInteractionOptions(2) {
            center.press(forDuration: 0.05, thenDragTo: center.withOffset(CGVector(dx: 0, dy: 150)), withVelocity: .fast, thenHoldForDuration: 0)
        }
        XCTAssertTrue(keyboard.disappears(within: 10), app.debugDescription)
        XCTAssertTrue(typed.exists, app.debugDescription)
    }

    func testALongMacDraftScrollsAndCopiesWithoutTheKeyboard() throws {
        continueAfterFailure = false
        let app = try demoApp("attachment-demo")
        app.launchEnvironment["COLLIE_DEMO_MAC_DRAFT"] = (1...12).map { "Line \($0) of the draft left in Claude Code on the Mac" }.joined(separator: "\n")
        try app.launch(onSession: "term_demo")
        defer { app.terminate() }
        let field = app.textFields.matching(NSPredicate(format: "value BEGINSWITH %@", "Line 1 ")).firstMatch
        XCTAssertTrue(field.appears(within: 10), app.debugDescription)
        XCTAssertGreaterThan(field.frame.height, 80, app.debugDescription)

        let before = field.screenshot().pngRepresentation
        field.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.8))
            .press(forDuration: 0.05, thenDragTo: field.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.1)))
        let keyboard = app.keyboards.firstMatch
        XCTAssertFalse(keyboard.exists, app.debugDescription)
        XCTAssertNotEqual(field.screenshot().pngRepresentation, before)

        field.press(forDuration: 1)
        XCTAssertTrue(app.element(Match(.any, "Copied")).appears(within: 2), app.debugDescription)
        XCTAssertFalse(keyboard.exists, app.debugDescription)

        field.tap()
        XCTAssertTrue(keyboard.appears(within: 10), app.debugDescription)
    }
}
