import XCTest

@MainActor
final class NewTaskPromptTests: XCTestCase {
    func testHideKeyboardPreservesThePrompt() throws {
        continueAfterFailure = false
        let app = try demoApp("attachment-demo")
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
        XCTAssertTrue(folderHint(in: app).appears(within: 5), app.debugDescription)
        preservedPrompt.tap()
        XCTAssertTrue(app.keyboards.firstMatch.appears(within: 5), app.debugDescription)
        XCTAssertTrue(preservedPrompt.exists, app.debugDescription)
    }

    func testChoosingARecentFolderKeepsTheFormsGeometry() throws {
        continueAfterFailure = false
        let app = try demoApp("screenshot-demo")
        app.launchEnvironment["COLLIE_DEMO_SCREENSHOTS"] = "1"
        app.launch()
        defer { app.terminate() }
        let newTask = app.buttons["New task"]
        XCTAssertTrue(newTask.appears(within: 10), app.debugDescription)
        newTask.tap()
        let prompt = app.textFields["What should the agent do?"]
        XCTAssertTrue(prompt.appears(within: 5), app.debugDescription)
        prompt.tap()
        XCTAssertTrue(app.keyboards.firstMatch.appears(within: 10), app.debugDescription)
        _ = app.type("go", into: prompt)
        app.buttons["Hide keyboard"].tap()
        XCTAssertTrue(app.keyboards.firstMatch.disappears(within: 5), app.debugDescription)
        let hint = folderHint(in: app)
        XCTAssertTrue(hint.appears(within: 5), app.debugDescription)
        let recent = app.buttons.matching(NSPredicate(format: "label CONTAINS %@", "/Users/demo/collie")).firstMatch
        let agent = app.buttons.matching(NSPredicate(format: "label BEGINSWITH %@", "Agent,")).firstMatch
        XCTAssertTrue(agent.appears(within: 5), app.debugDescription)
        let (rowHeight, agentY) = (recent.frame.height, agent.frame.minY)
        recent.tap()
        XCTAssertTrue(hint.disappears(within: 5), app.debugDescription)
        XCTAssertTrue(recent.exists, app.debugDescription)
        XCTAssertEqual(recent.frame.height, rowHeight, "the chosen recent folder keeps its path line")
        XCTAssertEqual(agent.frame.minY, agentY, "choosing a folder keeps the form's geometry")
    }

    private func folderHint(in app: XCUIApplication) -> XCUIElement {
        app.textFields.matching(NSPredicate(format: "placeholderValue == %@", "Choose a folder to start")).firstMatch
    }
}
