import XCTest

@MainActor
final class MachineConnectionTests: XCTestCase {
    func testErrorsAppearOnlyInMachinesAndCopyExactly() throws {
        continueAfterFailure = false
        let fixture = try XCTUnwrap(Bundle(for: Self.self).url(forResource: "attachment-demo", withExtension: "txt"))
        let message = "collie-linux.example.ts.net is not in this tailnet's netmap\nRetry failed."
        let app = XCUIApplication()
        app.launchArguments = [
            "-AppleLanguages", "(en)", "-AppleLocale", "en_US",
            "--terminal-demo", fixture.path
        ]
        app.launchEnvironment["COLLIE_DEMO_CONNECTION_ERROR"] = message
        app.launch()
        defer { app.terminate() }
        XCTAssertTrue(app.buttons["Layout"].appears(within: 10), app.debugDescription)
        for layout in ["Inbox", "List", "Grid"] {
            app.choose(app.buttons[layout]) { app.buttons["Layout"].tap() }
            XCTAssertFalse(app.buttons["Copy connection error"].exists, app.debugDescription)
            XCTAssertFalse(app.staticTexts[message].exists, app.debugDescription)
            XCTAssertFalse(app.staticTexts["Connection details"].exists)
        }
        app.buttons["Machines"].tap()
        let copy = app.buttons["Copy connection error"]
        XCTAssertTrue(copy.appears(within: 5), app.debugDescription)
        XCTAssertTrue(app.staticTexts[message].exists, app.debugDescription)
        XCTAssertFalse(app.staticTexts["Connection details"].exists)
        XCTAssertGreaterThanOrEqual(copy.frame.width, 44 - 0.001)
        XCTAssertGreaterThanOrEqual(copy.frame.height, 44 - 0.001)
        copy.tap()
        XCTAssertTrue(app.buttons["Copy connection error"].appears(within: 5))
        XCTAssertFalse(app.staticTexts["Copied"].exists)
        XCTAssertTrue(app.staticTexts[message].exists)
        XCTAssertFalse(app.staticTexts["Machine settings"].exists)
        let screenshot = XCTAttachment(screenshot: app.screenshot())
        screenshot.name = "machine-connection-error"
        screenshot.lifetime = .keepAlways
        add(screenshot)
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
