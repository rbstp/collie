import XCTest

@MainActor
final class SampleWorkspaceTests: XCTestCase {
    func testGridSubagentsAreAccessibleAndCappedWithoutCoveringBlockedAgents() throws {
        continueAfterFailure = false
        let app = XCUIApplication()
        let fixture = try XCTUnwrap(Bundle(for: Self.self).url(forResource: "screenshot-demo", withExtension: "txt"))
        app.launchArguments = ["-AppleLanguages", "(en)", "-AppleLocale", "en_US", "--terminal-demo", fixture.path, "--subagent-demo"]
        app.launch()
        defer { app.terminate() }
        let layout = app.buttons["Layout"]
        XCTAssertTrue(layout.appears(within: 10))
        app.choose(app.buttons["Grid"]) { layout.tap() }
        let card = app.buttons.containing(NSPredicate(format: "label CONTAINS %@", "fix the build")).firstMatch
        XCTAssertTrue(card.appears(within: 5))
        let label = card.label
        XCTAssertNotNil(label.range(
            of: #"Review approval flow, [0-9]+(?:h [0-9]+)?m, Run approval tests, [0-9]+(?:h [0-9]+)?m"#,
            options: .regularExpression
        ))
        XCTAssertTrue(label.contains("+3 more"))
        XCTAssertFalse(label.contains("Check retry handling"))
        XCTAssertFalse(label.contains("5 subagents"))
        XCTAssertFalse(label.contains("12k"))
        let blocked = app.buttons.containing(NSPredicate(format: "label CONTAINS %@", "add approval tests")).firstMatch
        XCTAssertTrue(blocked.exists)
        let blockedLabel = blocked.label
        XCTAssertFalse(blockedLabel.contains("Review deny path"))
        XCTAssertTrue(blockedLabel.contains("1 subagent"))

        app.choose(app.buttons["Star"]) { card.press(forDuration: 1) }
        let expanded = app.buttons.containing(NSPredicate(format: "label CONTAINS %@ AND label CONTAINS %@", "fix the build", "+1 more")).firstMatch
        XCTAssertTrue(expanded.appears(within: 5))
        let expandedLabel = card.label
        XCTAssertTrue(expandedLabel.contains("Check retry handling"))
        XCTAssertTrue(expandedLabel.contains("Inspect test fixtures"))
        XCTAssertTrue(expandedLabel.contains("+1 more"))
        XCTAssertFalse(expandedLabel.contains("Review documentation"))
    }

    func testSampleEntryResetAndExitWithoutDeveloperArguments() {
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
