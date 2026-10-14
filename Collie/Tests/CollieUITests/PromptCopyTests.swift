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
        XCTAssertTrue(keyboard.appears(within: 5), app.debugDescription)
        field.typeText("copy me")
        app.buttons["Hide keyboard"].tap()
        XCTAssertTrue(keyboard.disappears(within: 5), app.debugDescription)

        let typed = app.textFields.matching(NSPredicate(format: "value == %@", "copy me")).firstMatch
        typed.press(forDuration: 1)
        XCTAssertTrue(app.descendants(matching: .any)["Copied"].appears(within: 2), app.debugDescription)
        XCTAssertFalse(keyboard.exists, app.debugDescription)
        XCTAssertTrue(app.descendants(matching: .any)["Copied"].disappears(within: 5), app.debugDescription)

        typed.tap()
        XCTAssertTrue(keyboard.appears(within: 5), app.debugDescription)
    }
}
