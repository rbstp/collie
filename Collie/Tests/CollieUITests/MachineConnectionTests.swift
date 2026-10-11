import UIKit
import XCTest

@MainActor
final class MachineConnectionTests: XCTestCase {
    func testErrorsAppearOnlyInMachinesAndCopyExactly() throws {
        continueAfterFailure = false
        let fixture = try XCTUnwrap(Bundle(for: Self.self).url(forResource: "attachment-demo", withExtension: "txt"))
        let previousPasteboard = UIPasteboard.general.items
        defer { UIPasteboard.general.items = previousPasteboard }
        for message in [
            "collie-omarchy.tail76e46a.ts.net is not in this tailnet's netmap\nRetry failed.",
            "A different connection error"
        ] {
            let app = XCUIApplication()
            app.launchArguments = [
                "-AppleLanguages", "(en)", "-AppleLocale", "en_US",
                "--terminal-demo", fixture.path
            ]
            app.launchEnvironment["COLLIE_DEMO_CONNECTION_ERROR"] = message
            app.launch()
            defer { app.terminate() }
            for layout in ["Grid", "List", "Inbox"] {
                app.buttons["Layout"].tap()
                app.buttons[layout].tap()
                XCTAssertFalse(app.buttons["Copy connection error"].exists, app.debugDescription)
                XCTAssertFalse(app.staticTexts[message].exists, app.debugDescription)
                XCTAssertFalse(app.staticTexts["Connection details"].exists)
            }
            app.buttons["Machines"].tap()
            let copy = app.buttons["Copy connection error"]
            XCTAssertTrue(copy.waitForExistence(timeout: 5), app.debugDescription)
            XCTAssertTrue(app.staticTexts[message].exists, app.debugDescription)
            XCTAssertFalse(app.staticTexts["Connection details"].exists)
            XCTAssertGreaterThanOrEqual(copy.frame.width, 44)
            XCTAssertGreaterThanOrEqual(copy.frame.height, 44)
            copy.tap()
            XCTAssertEqual(UIPasteboard.general.string, message)
            XCTAssertTrue(app.staticTexts[message].exists)
            XCTAssertFalse(app.staticTexts["Machine settings"].exists)
            let screenshot = XCTAttachment(screenshot: app.screenshot())
            screenshot.name = "machine-connection-error"
            screenshot.lifetime = .keepAlways
            add(screenshot)
        }
    }
}
