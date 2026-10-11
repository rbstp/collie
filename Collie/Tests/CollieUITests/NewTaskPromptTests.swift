import XCTest

@MainActor
final class NewTaskPromptTests: XCTestCase {
    func testHideKeyboardPreservesThePrompt() throws {
        continueAfterFailure = false
        let app = XCUIApplication()
        let fixture = try XCTUnwrap(Bundle(for: Self.self).url(forResource: "attachment-demo", withExtension: "txt"))
        app.launchArguments = ["-AppleLanguages", "(en)", "-AppleLocale", "en_US", "--terminal-demo", fixture.path]
        app.launch()
        defer { app.terminate() }
        let newTask = app.buttons["New task"]
        XCTAssertTrue(newTask.waitForExistence(timeout: 10), app.debugDescription)
        newTask.tap()
        let prompt = app.textFields["What should the agent do?"]
        XCTAssertTrue(prompt.waitForExistence(timeout: 5), app.debugDescription)
        prompt.tap()
        XCTAssertTrue(app.keyboards.firstMatch.waitForExistence(timeout: 5), app.debugDescription)
        prompt.typeText("keep this prompt")
        let collapse = app.buttons["Hide keyboard"]
        XCTAssertTrue(collapse.waitForExistence(timeout: 5), app.debugDescription)
        XCTAssertTrue(collapse.isHittable, app.debugDescription)
        let screenshot = XCTAttachment(screenshot: app.screenshot())
        screenshot.name = "new-task-keyboard-collapse"
        screenshot.lifetime = .keepAlways
        add(screenshot)
        collapse.tap()
        XCTAssertTrue(app.keyboards.firstMatch.waitForNonExistence(timeout: 5), app.debugDescription)
        let preservedPrompt = app.descendants(matching: .any).matching(NSPredicate(format: "value == %@", "keep this prompt")).firstMatch
        XCTAssertTrue(preservedPrompt.waitForExistence(timeout: 5), app.debugDescription)
        preservedPrompt.tap()
        XCTAssertTrue(app.keyboards.firstMatch.waitForExistence(timeout: 5), app.debugDescription)
        XCTAssertTrue(preservedPrompt.exists, app.debugDescription)
    }
}
