import XCTest

@MainActor
final class PlanUsageWarningTests: XCTestCase {
    func testNearTheLimitShowsTheSelectedAgentsUsage() throws {
        continueAfterFailure = false
        let app = try openNewTask(used: "92")
        defer { app.terminate() }
        XCTAssertTrue(warning("Claude 5-hour limit at 92%, resets at ", in: app).appears(within: 5), app.debugDescription)
        pick("Codex", in: app)
        XCTAssertTrue(warning("Codex monthly limit at 92%, resets ", in: app).appears(within: 5), app.debugDescription)
        XCTAssertFalse(warning("Claude 5-hour limit", in: app).exists)
        pick("Copilot", in: app)
        XCTAssertTrue(anyWarning(in: app).disappears(within: 5), app.debugDescription)
    }

    func testAtTheLimitWarnsWithoutBlockingStart() throws {
        continueAfterFailure = false
        let app = try openNewTask(used: "100")
        defer { app.terminate() }
        let reached = warning("Claude 5-hour limit reached, resets at ", in: app)
        XCTAssertTrue(reached.appears(within: 5), app.debugDescription)
        pick("Codex", in: app)
        XCTAssertTrue(warning("Codex monthly limit reached, resets ", in: app).appears(within: 5), app.debugDescription)
        pick("Claude", in: app)
        XCTAssertTrue(reached.appears(within: 5), app.debugDescription)
        app.buttons.matching(NSPredicate(format: "label CONTAINS %@", "/Users/demo/collie")).firstMatch.tap()
        let prompt = app.textFields["What should the agent do?"]
        prompt.tap()
        _ = app.type("go", into: prompt)
        app.buttons["Hide keyboard"].tap()
        let start = app.buttons["Start"]
        for _ in 0..<3 where !start.isHittable { app.swipeUp() }
        XCTAssertTrue(start.isEnabled, app.debugDescription)
    }

    func testBelowTheLimitOrWithoutAReadingShowsNothing() throws {
        continueAfterFailure = false
        let app = try openNewTask(used: nil)
        defer { app.terminate() }
        for agent in ["Codex", "Claude"] {
            pick(agent, in: app)
            XCTAssertFalse(anyWarning(in: app).exists, app.debugDescription)
        }
    }

    private func openNewTask(used: String?) throws -> XCUIApplication {
        let app = try demoApp("screenshot-demo")
        app.launchEnvironment["COLLIE_DEMO_SCREENSHOTS"] = "1"
        if let used { app.launchEnvironment["COLLIE_DEMO_PLAN_USED"] = used }
        app.launch()
        let newTask = app.buttons["New task"]
        XCTAssertTrue(newTask.appears(within: 10), app.debugDescription)
        newTask.tap()
        XCTAssertTrue(agentPicker(in: app).appears(within: 5), app.debugDescription)
        return app
    }

    private func pick(_ agent: String, in app: XCUIApplication) {
        app.choose(app.buttons[agent]) { agentPicker(in: app).tap() }
    }

    /// The picker's label carries its value, such as "Agent, Claude".
    private func agentPicker(in app: XCUIApplication) -> XCUIElement {
        app.buttons.matching(NSPredicate(format: "label BEGINSWITH %@", "Agent,")).firstMatch
    }

    private func warning(_ prefix: String, in app: XCUIApplication) -> XCUIElement {
        app.staticTexts.matching(NSPredicate(format: "label BEGINSWITH %@", prefix)).firstMatch
    }

    private func anyWarning(in app: XCUIApplication) -> XCUIElement {
        app.staticTexts.matching(NSPredicate(format: "label CONTAINS %@ OR label CONTAINS %@", " limit at ", " limit reached")).firstMatch
    }
}
