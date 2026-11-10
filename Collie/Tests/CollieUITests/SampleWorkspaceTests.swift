import XCTest

@MainActor
final class SampleWorkspaceTests: XCTestCase {
    func testSampleEntryActionsResetAndExitWithoutDeveloperArguments() {
        let app = XCUIApplication()
        app.launch()
        defer { app.terminate() }
        if app.buttons["Settings"].exists { app.buttons["Settings"].tap() }
        let explore = app.buttons["Explore sample workspace"]
        XCTAssertTrue(explore.appears(within: 10))
        explore.tap()
        XCTAssertTrue(app.staticTexts["Simulated actions only"].appears(within: 5))
        app.choose(app.buttons["List"]) { app.buttons["Layout"].tap() }
        let blocked = app.staticTexts["add approval tests"].firstMatch
        XCTAssertTrue(blocked.appears(within: 5))
        blocked.tap()
        let deny = app.buttons["Deny"]
        XCTAssertTrue(deny.appears(within: 5))
        deny.tap()
        XCTAssertTrue(app.staticTexts["Simulated denial. No command was run."].appears(within: 10))
        app.buttons["Terminal"].tap()
        app.buttons["Keys"].tap()
        let escape = app.buttons["Escape"]
        XCTAssertTrue(escape.appears(within: 5))
        escape.tap()
        app.buttons["Chat"].tap()
        XCTAssertTrue(app.staticTexts["Simulated terminal keys received. No keys were sent to a machine."].appears(within: 10))
        let prompt = app.textFields["Prompt the agent"]
        XCTAssertTrue(prompt.appears(within: 5))
        prompt.tap()
        XCTAssertTrue(app.keyboards.firstMatch.appears(within: 10))
        _ = app.type("Show sample result", into: prompt)
        app.buttons["Send"].tap()
        XCTAssertTrue(app.staticTexts.containing(NSPredicate(format: "label CONTAINS %@", "Simulated response to: Show sample result")).firstMatch.appears(within: 10))

        app.choose(app.buttons["Git changes"]) { app.buttons["More"].tap() }
        XCTAssertTrue(app.navigationBars["Git changes"].appears(within: 10))
        let row = app.buttons.containing(NSPredicate(format: "label CONTAINS %@", "sample-approval-tests")).firstMatch
        XCTAssertTrue(row.appears(within: 10))
        XCTAssertFalse(app.buttons["Select"].exists, "the session's own checkout is the only worktree")
        row.tap()
        XCTAssertTrue(app.navigationBars["sample-approval-tests"].appears(within: 5))
        app.navigationBars.buttons.element(boundBy: 0).tap()
        app.buttons["Done"].tap()

        app.navigationBars.buttons.element(boundBy: 0).tap()
        app.buttons["Machines"].tap()
        XCTAssertTrue(app.staticTexts["MacBook Pro"].appears(within: 5))
        app.buttons["Settings"].tap()
        app.buttons["Reset sample workspace"].tap()
        XCTAssertTrue(app.staticTexts["Simulated actions only"].appears(within: 5))
        app.buttons["Exit"].tap()
        XCTAssertTrue(explore.appears(within: 5))
    }
}
