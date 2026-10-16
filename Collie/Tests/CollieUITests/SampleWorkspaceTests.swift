import XCTest

@MainActor
final class SampleWorkspaceTests: XCTestCase {
    func testSampleEntryResetAndExitWithoutDeveloperArguments() {
        let app = XCUIApplication()
        app.launch()
        defer { app.terminate() }
        if app.buttons["Settings"].exists { app.buttons["Settings"].tap() }
        let explore = app.buttons["Explore sample workspace"]
        XCTAssertTrue(explore.appears(within: 10))
        explore.tap()
        XCTAssertTrue(app.staticTexts["Simulated actions only"].appears(within: 5))
        app.buttons["Layout"].tap()
        app.buttons["List"].tap()
        let blocked = app.staticTexts["add approval tests"].firstMatch
        XCTAssertTrue(blocked.appears(within: 5))
        blocked.tap()
        let deny = app.buttons["Deny"]
        XCTAssertTrue(deny.appears(within: 5))
        deny.tap()
        XCTAssertTrue(app.staticTexts["Simulated denial. No command was run."].appears(within: 10))
        app.buttons["Terminal"].tap()
        app.buttons["Keys"].tap()
        app.buttons["Escape"].tap()
        app.buttons["Chat"].tap()
        XCTAssertTrue(app.staticTexts["Simulated terminal keys received. No keys were sent to a machine."].appears(within: 10))
        let prompt = app.textFields["Prompt the agent"]
        prompt.tap()
        prompt.typeText("Show sample result")
        app.buttons["Send"].tap()
        XCTAssertTrue(app.staticTexts.containing(NSPredicate(format: "label CONTAINS %@", "Simulated response to: Show sample result")).firstMatch.appears(within: 10))
        let screenshot = XCTAttachment(screenshot: app.screenshot())
        screenshot.name = "release-sample-actions"
        screenshot.lifetime = .keepAlways
        add(screenshot)
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
