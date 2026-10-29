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
        XCTAssertTrue(warning("Use the session's terminal buttons", in: app).appears(within: 5), app.debugDescription)
        XCTAssertFalse(anyWarning(in: app).exists)
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
        app.buttons["/Users/demo/collie"].tap()
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
        let hint = warning("Use the session's terminal buttons", in: app)
        for agent in ["Codex", "Claude"] {
            pick(agent, in: app)
            XCTAssertTrue(hint.appears(within: 5), app.debugDescription)
            XCTAssertFalse(anyWarning(in: app).exists)
        }
    }

    private func openNewTask(used: String?) throws -> XCUIApplication {
        let app = XCUIApplication()
        let fixture = try XCTUnwrap(Bundle(for: Self.self).url(forResource: "screenshot-demo", withExtension: "txt"))
        app.launchArguments = ["-AppleLanguages", "(en)", "-AppleLocale", "en_US", "--terminal-demo", fixture.path]
        app.launchEnvironment["COLLIE_DEMO_SCREENSHOTS"] = "1"
        if let used { app.launchEnvironment["COLLIE_DEMO_PLAN_USED"] = used }
        app.launch()
        let newTask = app.buttons["New task"]
        XCTAssertTrue(newTask.appears(within: 10), app.debugDescription)
        newTask.tap()
        XCTAssertTrue(app.buttons["Agent"].appears(within: 5), app.debugDescription)
        return app
    }

    private func pick(_ agent: String, in app: XCUIApplication) {
        app.choose(app.buttons[agent]) { app.buttons["Agent"].tap() }
    }

    private func warning(_ prefix: String, in app: XCUIApplication) -> XCUIElement {
        app.staticTexts.matching(NSPredicate(format: "label BEGINSWITH %@", prefix)).firstMatch
    }

    private func anyWarning(in app: XCUIApplication) -> XCUIElement {
        app.staticTexts.matching(NSPredicate(format: "label CONTAINS %@ OR label CONTAINS %@", " limit at ", " limit reached")).firstMatch
    }
}
