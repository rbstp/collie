import XCTest

@MainActor
final class PromptCopyTests: XCTestCase {
    func testLongPressCopiesThePromptWithoutTheKeyboard() throws {
        continueAfterFailure = false
        let app = XCUIApplication()
        app.launchArguments = ["-AppleLanguages", "(en)", "-AppleLocale", "en_US"]
        let fixture = try XCTUnwrap(Bundle(for: Self.self).url(forResource: "attachment-demo", withExtension: "txt"))
        app.launchArguments += ["--terminal-demo", fixture.path]
        app.launch()
        defer { app.terminate() }
        app.open(try XCTUnwrap(URL(string: "collie://agent?m=demo&t=term_demo")))
        let field = app.textFields["Prompt the agent"]
        XCTAssertTrue(field.appears(within: 10), app.debugDescription)
        field.tap()
        let keyboard = app.keyboards.firstMatch
        XCTAssertTrue(keyboard.appears(within: 10), app.debugDescription)
        let typed = app.type("copy me", into: field)
        app.buttons["Hide keyboard"].tap()
        XCTAssertTrue(keyboard.disappears(within: 10), app.debugDescription)

        typed.press(forDuration: 1)
        let copied = app.element(Match(.any, "Copied"))
        XCTAssertTrue(copied.appears(within: 2), app.debugDescription)
        XCTAssertFalse(keyboard.exists, app.debugDescription)
        XCTAssertTrue(copied.disappears(within: 5), app.debugDescription)

        typed.tap()
        XCTAssertTrue(keyboard.appears(within: 10), app.debugDescription)
    }

    func testDragScrollsALongMacDraftWithoutTheKeyboard() throws {
        continueAfterFailure = false
        let app = XCUIApplication()
        app.launchArguments = ["-AppleLanguages", "(en)", "-AppleLocale", "en_US"]
        let fixture = try XCTUnwrap(Bundle(for: Self.self).url(forResource: "attachment-demo", withExtension: "txt"))
        app.launchArguments += ["--terminal-demo", fixture.path]
        app.launchEnvironment["COLLIE_DEMO_MAC_DRAFT"] = (1...12).map { "Line \($0) of the draft left in Claude Code on the Mac" }.joined(separator: "\n")
        app.launch()
        defer { app.terminate() }
        app.open(try XCTUnwrap(URL(string: "collie://agent?m=demo&t=term_demo")))
        let field = app.textFields.matching(NSPredicate(format: "value BEGINSWITH %@", "Line 1 ")).firstMatch
        XCTAssertTrue(field.appears(within: 10), app.debugDescription)
        XCTAssertGreaterThan(field.frame.height, 80, app.debugDescription)

        let before = field.screenshot().pngRepresentation
        field.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.8))
            .press(forDuration: 0.05, thenDragTo: field.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.1)))
        let keyboard = app.keyboards.firstMatch
        XCTAssertFalse(keyboard.appears(within: 1), app.debugDescription)
        XCTAssertNotEqual(field.screenshot().pngRepresentation, before)

        field.press(forDuration: 1)
        XCTAssertTrue(app.element(Match(.any, "Copied")).appears(within: 2), app.debugDescription)
        XCTAssertFalse(keyboard.exists, app.debugDescription)

        field.tap()
        XCTAssertTrue(keyboard.appears(within: 10), app.debugDescription)
    }
}
