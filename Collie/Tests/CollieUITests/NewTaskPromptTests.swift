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
        XCTAssertTrue(newTask.appears(within: 10), app.debugDescription)
        newTask.tap()
        let prompt = app.textFields["What should the agent do?"]
        XCTAssertTrue(prompt.appears(within: 5), app.debugDescription)
        prompt.tap()
        XCTAssertTrue(app.keyboards.firstMatch.appears(within: 10), app.debugDescription)
        let preservedPrompt = app.type("keep this prompt", into: prompt)
        let collapse = app.buttons["Hide keyboard"]
        XCTAssertTrue(collapse.appears(within: 5), app.debugDescription)
        XCTAssertTrue(eventually { collapse.isHittable }, app.debugDescription)
        let screenshot = XCTAttachment(screenshot: app.screenshot())
        screenshot.name = "new-task-keyboard-collapse"
        screenshot.lifetime = .keepAlways
        add(screenshot)
        collapse.tap()
        XCTAssertTrue(app.keyboards.firstMatch.disappears(within: 5), app.debugDescription)
        XCTAssertTrue(preservedPrompt.appears(within: 5), app.debugDescription)
        preservedPrompt.tap()
        XCTAssertTrue(app.keyboards.firstMatch.appears(within: 5), app.debugDescription)
        XCTAssertTrue(preservedPrompt.exists, app.debugDescription)
    }
}
