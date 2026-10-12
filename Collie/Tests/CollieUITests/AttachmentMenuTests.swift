import XCTest

@MainActor
final class AttachmentMenuTests: XCTestCase {
    func testSavedPromptsInNewTask() throws { try checkChoice("Saved prompts", newTask: true) }
    func testFilesInNewTask() throws { try checkChoice("Files", newTask: true) }
    func testPhotosInNewTask() throws { try checkChoice("Photo Library", newTask: true) }
    func testSavedPromptsInSession() throws { try checkChoice("Saved prompts", newTask: false) }
    func testFilesInSession() throws { try checkChoice("Files", newTask: false) }
    func testPhotosInSession() throws { try checkChoice("Photo Library", newTask: false) }

    private func launchDemo() throws -> XCUIApplication {
        continueAfterFailure = false
        let app = XCUIApplication()
        app.launchArguments = ["-AppleLanguages", "(en)", "-AppleLocale", "en_US"]
        let fixture = try XCTUnwrap(Bundle(for: Self.self).url(forResource: "attachment-demo", withExtension: "txt"))
        app.launchArguments += [
            "--terminal-demo",
            fixture.path
        ]
        app.launch()
        XCTAssertTrue(app.buttons["New task"].waitForExistence(timeout: 10), app.debugDescription)
        return app
    }

    private func checkChoice(_ choice: String, newTask: Bool) throws {
        let app = try launchDemo()
        defer { app.terminate() }
        if newTask {
            app.buttons["New task"].tap()
        } else {
            app.buttons["Layout"].tap()
            app.buttons["Grid"].tap()
            let agent = app.buttons.matching(NSPredicate(format: "label CONTAINS %@", "fix the build")).firstMatch
            XCTAssertTrue(agent.waitForExistence(timeout: 5), app.debugDescription)
            agent.tap()
        }
        for keyboard in [false, true] {
            if keyboard {
                let field = app.textFields[newTask ? "What should the agent do?" : "Prompt the agent"]
                field.tap()
                XCTAssertTrue(app.keyboards.firstMatch.waitForExistence(timeout: 5), app.debugDescription)
                field.typeText("test")
            }
            let attachment = app.buttons["Attachments and saved prompts"]
            XCTAssertTrue(attachment.waitForExistence(timeout: 5), app.debugDescription)
            let screenshot = XCTAttachment(screenshot: app.screenshot())
            screenshot.name = "\(name)-keyboard-\(keyboard)"
            screenshot.lifetime = .keepAlways
            add(screenshot)
            attachment.tap()
            let option = app.buttons[choice].firstMatch
            XCTAssertTrue(option.waitForExistence(timeout: 3), app.debugDescription)
            option.tap()
            let picker = switch choice {
            case "Saved prompts": app.navigationBars["Saved prompts"]
            case "Files": app.navigationBars["FullDocumentManagerViewControllerNavigationBar"]
            default: app.navigationBars["Photos"]
            }
            // Cancel remote pickers before another presentation or terminating their host app.
            XCTAssertTrue(picker.waitForExistence(timeout: choice == "Saved prompts" ? 5 : 30), app.debugDescription)
            let cancel = picker.buttons["Cancel"]
            XCTAssertTrue(cancel.waitForExistence(timeout: 5), app.debugDescription)
            cancel.tap()
            XCTAssertTrue(picker.waitForNonExistence(timeout: 5), app.debugDescription)
        }
    }
}
