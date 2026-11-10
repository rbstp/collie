import XCTest

@MainActor
final class NewTaskPromptTests: XCTestCase {
    func testNewTaskWarnsAtThePlanLimitKeepsThePromptAndNamesTheWorkspace() throws {
        continueAfterFailure = false
        let app = try demoApp("screenshot-demo")
        app.launchEnvironment["COLLIE_DEMO_SCREENSHOTS"] = "1"
        app.launchEnvironment["COLLIE_DEMO_PLAN_USED"] = "100"
        app.launch()
        defer { app.terminate() }
        let newTask = app.buttons["New task"]
        XCTAssertTrue(newTask.appears(within: 10), app.debugDescription)
        newTask.tap()
        // The picker's label carries its value, such as "Agent, Claude".
        let agent = app.buttons.matching(NSPredicate(format: "label BEGINSWITH %@", "Agent,")).firstMatch
        XCTAssertTrue(agent.appears(within: 5), app.debugDescription)
        let warning = { (prefix: String) in app.staticTexts.matching(NSPredicate(format: "label BEGINSWITH %@", prefix)).firstMatch }
        let claude = warning("Claude 5-hour limit reached, resets at ")
        XCTAssertTrue(claude.appears(within: 5), app.debugDescription)
        app.choose(app.buttons["Codex"]) { agent.tap() }
        XCTAssertTrue(warning("Codex monthly limit reached, resets ").appears(within: 5), app.debugDescription)
        XCTAssertFalse(claude.exists, app.debugDescription)

        let prompt = app.textFields["What should the agent do?"]
        prompt.tap()
        let keyboard = app.keyboards.firstMatch
        XCTAssertTrue(keyboard.appears(within: 10), app.debugDescription)
        let typed = app.type("keep this prompt", into: prompt)
        let collapse = app.buttons["Hide keyboard"]
        XCTAssertTrue(eventually { collapse.exists && collapse.isHittable }, app.debugDescription)
        collapse.tap()
        XCTAssertTrue(keyboard.disappears(within: 5), app.debugDescription)
        XCTAssertTrue(typed.exists, app.debugDescription)
        let hint = app.textFields.matching(NSPredicate(format: "placeholderValue == %@", "Choose a folder to start")).firstMatch
        XCTAssertTrue(hint.appears(within: 5), app.debugDescription)

        let recent = app.buttons.matching(NSPredicate(format: "label CONTAINS %@", "/Users/demo/collie")).firstMatch
        let (rowHeight, agentY) = (recent.frame.height, agent.frame.minY)
        recent.tap()
        XCTAssertTrue(hint.disappears(within: 5), app.debugDescription)
        XCTAssertTrue(recent.exists, app.debugDescription)
        XCTAssertEqual(recent.frame.height, rowHeight, "the chosen recent folder keeps its path line")
        XCTAssertEqual(agent.frame.minY, agentY, "choosing a folder keeps the form's geometry")

        typed.tap()
        XCTAssertTrue(keyboard.appears(within: 5), app.debugDescription)
        // A gesture while the field still moves misses it: the form scrolls its prompt above the keyboard after the keyboard appears.
        XCTAssertNotNil(typed.settledFrame(), app.debugDescription)
        // XCUIElement.swipeDown travels a fraction of the element, too short on a one-line field for a swipe.
        let center = typed.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5))
        center.press(forDuration: 0.05, thenDragTo: center.withOffset(CGVector(dx: 0, dy: 150)), withVelocity: .fast, thenHoldForDuration: 0)
        XCTAssertTrue(keyboard.disappears(within: 10), app.debugDescription)
        XCTAssertTrue(typed.exists, app.debugDescription)
        // At the limit the warning never blocks Start.
        XCTAssertTrue(app.buttons["Start"].isEnabled, app.debugDescription)

        let name = app.buttons["Name workspace"]
        XCTAssertTrue(name.appears(within: 5), app.debugDescription)
        let label = app.textFields["Workspace label"]
        XCTAssertFalse(label.exists, app.debugDescription)
        XCTAssertGreaterThanOrEqual(name.frame.height, 44, app.debugDescription)
        // Near the target's top edge, above the words.
        name.coordinate(withNormalizedOffset: .zero).withOffset(CGVector(dx: 30, dy: 3)).tap()
        XCTAssertTrue(label.appears(within: 5), app.debugDescription)
        XCTAssertTrue(name.disappears(within: 5), app.debugDescription)
        XCTAssertTrue(keyboard.appears(within: 10), app.debugDescription)
        // The whole row clears the keyboard, not only the field's text.
        XCTAssertTrue(eventually(within: 10) { (label.settledFrame()?.maxY ?? .infinity) + 8 <= keyboard.frame.minY }, app.debugDescription)
        _ = app.type("docs", into: label)
    }
}
