import XCTest

@MainActor
final class SuggestionTests: XCTestCase {
    private let suggestion = "run the tests"

    private func launch(draft: String? = nil) throws -> XCUIApplication {
        continueAfterFailure = false
        let app = try demoApp("landscape-demo")
        app.launchEnvironment["COLLIE_DEMO_SUGGESTION"] = suggestion
        if let draft { app.launchEnvironment["COLLIE_DEMO_MAC_DRAFT"] = draft }
        app.launch()
        XCTAssertTrue(app.buttons["New task"].appears(within: 10), app.debugDescription)
        app.open(try XCTUnwrap(URL(string: "collie://agent?m=demo&t=term_demo")))
        return app
    }

    func testTabShowsInTheBottomRowWhileASuggestionShows() throws {
        let app = try launch()
        defer { app.terminate() }
        let terminalView = app.buttons["Terminal"].firstMatch
        XCTAssertTrue(terminalView.appears(within: 10), app.debugDescription)
        terminalView.tap()

        let keys = app.buttons["Keys"]
        XCTAssertTrue(keys.appears(within: 5), app.debugDescription)
        let escape = app.buttons["Escape"]
        let accept = app.buttons["Accept suggestion"]
        if !keys.isSelected { keys.tap() }
        XCTAssertTrue(escape.appears(within: 5), app.debugDescription)
        XCTAssertFalse(accept.exists, "the strip's own Tab covers it")
        keys.tap()
        XCTAssertTrue(escape.disappears(within: 5), app.debugDescription)

        XCTAssertTrue(accept.appears(within: 5), app.debugDescription)
        let returnKey = app.buttons["Return"]
        XCTAssertTrue(accept.isHittable && returnKey.exists, app.debugDescription)
        XCTAssertEqual(accept.frame.midY, returnKey.frame.midY, accuracy: 2, app.debugDescription)
        XCTAssertLessThan(accept.frame.maxX, returnKey.frame.minX, app.debugDescription)
        let screenshot = XCTAttachment(screenshot: app.screenshot())
        screenshot.name = "suggestion-terminal"
        screenshot.lifetime = .keepAlways
        add(screenshot)
        accept.tap()
        XCTAssertTrue(accept.disappears(within: 5), app.debugDescription)
    }

    func testChatFillsAnEmptyPromptWithTheSuggestion() throws {
        let app = try launch()
        defer { app.terminate() }
        let shown = app.textFields.matching(NSPredicate(format: "placeholderValue == %@", suggestion)).firstMatch
        XCTAssertTrue(shown.appears(within: 10), app.debugDescription)
        let use = app.buttons["Use suggestion"]
        XCTAssertTrue(use.exists, app.debugDescription)
        let screenshot = XCTAttachment(screenshot: app.screenshot())
        screenshot.name = "suggestion-chat"
        screenshot.lifetime = .keepAlways
        add(screenshot)
        use.tap()
        XCTAssertTrue(use.disappears(within: 5), app.debugDescription)
        let filled = app.textFields.matching(NSPredicate(format: "value == %@", suggestion)).firstMatch
        XCTAssertTrue(filled.appears(within: 5), app.debugDescription)
        XCTAssertTrue(app.buttons["Send"].isEnabled, app.debugDescription)
    }

    func testASyncedDraftWinsOverTheSuggestion() throws {
        let app = try launch(draft: "typed on the Mac")
        defer { app.terminate() }
        let field = app.textFields.matching(NSPredicate(format: "value == %@", "typed on the Mac")).firstMatch
        XCTAssertTrue(field.appears(within: 10), app.debugDescription)
        XCTAssertFalse(app.buttons["Use suggestion"].exists, app.debugDescription)
    }
}
