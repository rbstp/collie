import XCTest

@MainActor
final class MachineConnectionTests: XCTestCase {
    func testErrorsAppearOnlyInMachinesAndCopyExactly() throws {
        continueAfterFailure = false
        let message = "collie-linux.example.ts.net is not in this tailnet's netmap\nRetry failed."
        let app = try demoApp("attachment-demo")
        app.launchEnvironment["COLLIE_DEMO_CONNECTION_ERROR"] = message
        app.launch()
        defer { app.terminate() }
        let layout = app.buttons["Layout"]
        XCTAssertTrue(layout.appears(within: 10), app.debugDescription)
        // The layout with a header for each machine, where the link's state shows.
        app.choose(app.buttons["List"]) { layout.tap() }
        XCTAssertFalse(app.buttons["Copy connection error"].exists, app.debugDescription)
        XCTAssertFalse(app.staticTexts[message].exists, app.debugDescription)
        app.buttons["Machines"].tap()
        let copy = app.buttons["Copy connection error"]
        XCTAssertTrue(copy.appears(within: 5), app.debugDescription)
        XCTAssertTrue(app.staticTexts[message].exists, app.debugDescription)
        XCTAssertGreaterThanOrEqual(copy.frame.width, 44 - 0.001)
        XCTAssertGreaterThanOrEqual(copy.frame.height, 44 - 0.001)
        copy.tap()
        app.buttons["Agents"].tap()
        app.buttons["New task"].tap()
        let prompt = app.textFields["What should the agent do?"]
        XCTAssertTrue(prompt.appears(within: 5), app.debugDescription)
        prompt.tap()
        prompt.press(forDuration: 1)
        let paste = app.menuItems["Paste"].exists ? app.menuItems["Paste"] : app.buttons["Paste"]
        XCTAssertTrue(paste.appears(within: 5), app.debugDescription)
        paste.tap()
        let pasted = app.element(Match(.any, NSPredicate(format: "value == %@", message)))
        XCTAssertTrue(pasted.appears(within: 5), app.debugDescription)
    }
}
