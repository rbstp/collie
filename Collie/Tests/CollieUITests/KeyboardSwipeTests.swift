import XCTest

@MainActor
final class KeyboardSwipeTests: XCTestCase {
    private func launch(draft: String? = nil) throws -> XCUIApplication {
        continueAfterFailure = false
        let app = XCUIApplication()
        let fixture = try XCTUnwrap(Bundle(for: Self.self).url(forResource: "attachment-demo", withExtension: "txt"))
        app.launchArguments = ["-AppleLanguages", "(en)", "-AppleLocale", "en_US", "--terminal-demo", fixture.path]
        if let draft { app.launchEnvironment["COLLIE_DEMO_MAC_DRAFT"] = draft }
        app.launch()
        return app
    }

    // XCUIElement.swipeDown travels a fraction of the element, too short on a one-line field for a swipe.
    private func swipeDown(on element: XCUIElement) {
        let start = element.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5))
        start.press(forDuration: 0.05, thenDragTo: start.withOffset(CGVector(dx: 0, dy: 150)), withVelocity: .fast, thenHoldForDuration: 0)
    }

    private func typeAndSwipeDown(_ field: XCUIElement, in app: XCUIApplication) {
        XCTAssertTrue(field.appears(within: 10), app.debugDescription)
        field.tap()
        let keyboard = app.keyboards.firstMatch
        XCTAssertTrue(keyboard.appears(within: 5), app.debugDescription)
        field.typeText("keep me")
        // The New task prompt grows vertically and can surface as a text view.
        let typed = app.descendants(matching: .any).matching(NSPredicate(format: "value == %@", "keep me")).firstMatch
        XCTAssertTrue(typed.appears(within: 5), app.debugDescription)
        // New task scrolls its prompt above the keyboard after the keyboard appears; a swipe during that scroll misses the field.
        var frame = CGRect.null
        for _ in 0..<10 where typed.frame != frame {
            frame = typed.frame
            Thread.sleep(forTimeInterval: 0.3)
        }
        swipeDown(on: typed)
        XCTAssertTrue(keyboard.disappears(within: 5), app.debugDescription)
        XCTAssertTrue(typed.exists, app.debugDescription)
    }

    func testSwipeDownOnTheSessionPromptDismissesTheKeyboard() throws {
        let app = try launch()
        defer { app.terminate() }
        app.open(try XCTUnwrap(URL(string: "collie://agent?m=demo&t=term_demo")))
        let field = app.textFields["Prompt the agent"]
        XCTAssertTrue(field.appears(within: 10), app.debugDescription)
        field.tap()
        let keyboard = app.keyboards.firstMatch
        XCTAssertTrue(keyboard.appears(within: 5), app.debugDescription)
        field.typeText("keep me")
        let typed = app.textFields.matching(NSPredicate(format: "value == %@", "keep me")).firstMatch
        typed.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5))
            .press(forDuration: 1, thenDragTo: typed.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 1.5)))
        XCTAssertFalse(keyboard.disappears(within: 1), app.debugDescription)
        typed.tap()
        swipeDown(on: typed)
        XCTAssertTrue(keyboard.disappears(within: 5), app.debugDescription)

        app.buttons["Terminal"].firstMatch.tap()
        XCTAssertTrue(typed.appears(within: 5), app.debugDescription)
        typed.tap()
        XCTAssertTrue(keyboard.appears(within: 5), app.debugDescription)
        swipeDown(on: typed)
        XCTAssertTrue(keyboard.disappears(within: 5), app.debugDescription)
    }

    func testSwipeDownOnTheInboxSearchDismissesTheKeyboard() throws {
        let app = try launch()
        defer { app.terminate() }
        let layout = app.buttons["Layout"]
        XCTAssertTrue(layout.appears(within: 10), app.debugDescription)
        app.choose(app.buttons["Inbox"]) { layout.tap() }
        typeAndSwipeDown(app.textFields.matching(NSPredicate(format: "placeholderValue == %@", "Search agents")).firstMatch, in: app)
    }

    func testSwipeDownOnTheNewTaskPromptDismissesTheKeyboard() throws {
        let app = try launch()
        defer { app.terminate() }
        let newTask = app.buttons["New task"]
        XCTAssertTrue(newTask.appears(within: 10), app.debugDescription)
        newTask.tap()
        typeAndSwipeDown(app.textFields["What should the agent do?"], in: app)
    }

    func testSwipeDownScrollsALongDraftBeforeDismissingTheKeyboard() throws {
        let app = try launch(draft: (1...12).map { "Line \($0) of the draft left in Claude Code on the Mac" }.joined(separator: "\n"))
        defer { app.terminate() }
        app.open(try XCTUnwrap(URL(string: "collie://agent?m=demo&t=term_demo")))
        let field = app.textFields.matching(NSPredicate(format: "value BEGINSWITH %@", "Line 1 ")).firstMatch
        XCTAssertTrue(field.appears(within: 10), app.debugDescription)
        field.tap()
        let keyboard = app.keyboards.firstMatch
        XCTAssertTrue(keyboard.appears(within: 5), app.debugDescription)
        // Focus can leave the caret at the start, and the field scrolls the caret back into view after a drag. A caret on
        // the last visible row keeps the draft below its top either way.
        field.coordinate(withNormalizedOffset: CGVector(dx: 0.3, dy: 0.9)).tap()
        for _ in 0..<3 {
            field.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.9))
                .press(forDuration: 0.05, thenDragTo: field.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.1)))
        }

        swipeDown(on: field)
        XCTAssertFalse(keyboard.disappears(within: 1), app.debugDescription)
        for _ in 0..<5 where keyboard.exists { swipeDown(on: field) }
        XCTAssertTrue(keyboard.disappears(within: 5), app.debugDescription)
    }
}
