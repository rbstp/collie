import XCTest

@MainActor
final class AttachmentMenuTests: XCTestCase {
    func testChoicesInNewTask() throws { try checkChoices(newTask: true) }
    func testChoicesInSession() throws { try checkChoices(newTask: false) }

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
        XCTAssertTrue(app.buttons["New task"].appears(within: 10), app.debugDescription)
        return app
    }

    private func checkChoices(newTask: Bool) throws {
        let app = try launchDemo()
        defer { app.terminate() }
        if newTask {
            app.buttons["New task"].tap()
        } else {
            app.buttons["Layout"].tap()
            app.buttons["Grid"].tap()
            let agent = app.buttons.matching(NSPredicate(format: "label CONTAINS %@", "fix the build")).firstMatch
            XCTAssertTrue(agent.appears(within: 5), app.debugDescription)
            agent.tap()
        }
        let placeholder = newTask ? "What should the agent do?" : "Prompt the agent"
        let field = app.textFields.matching(NSPredicate(format: "placeholderValue == %@ OR label == %@ OR value == %@", placeholder, placeholder, "test")).firstMatch
        for keyboard in [false, true] {
            for choice in ["Saved prompts", "Files", "Photo Library"] {
                XCTContext.runActivity(named: "\(choice), keyboard \(keyboard)") { _ in
                    present(choice, keyboard: keyboard, field: field, in: app)
                }
            }
        }
    }

    private func present(_ choice: String, keyboard: Bool, field: XCUIElement, in app: XCUIApplication) {
        if keyboard && !app.keyboards.firstMatch.exists {
            field.tap()
            XCTAssertTrue(app.keyboards.firstMatch.appears(within: 5), app.debugDescription)
            if field.value as? String != "test" {
                field.typeText("test")
            }
        }
        XCTAssertEqual(app.keyboards.firstMatch.exists, keyboard, app.debugDescription)
        let attachment = app.buttons["Attachments and saved prompts"]
        XCTAssertTrue(attachment.appears(within: 5), app.debugDescription)
        let screenshot = XCTAttachment(screenshot: app.screenshot())
        screenshot.name = "\(name)-\(choice)-keyboard-\(keyboard)"
        screenshot.lifetime = .keepAlways
        add(screenshot)
        attachment.tap()
        let option = app.buttons[choice].firstMatch
        XCTAssertTrue(option.appears(within: 3), app.debugDescription)
        option.tap()
        let picker = switch choice {
        case "Saved prompts": app.navigationBars["Saved prompts"]
        case "Files": app.navigationBars["FullDocumentManagerViewControllerNavigationBar"]
        // On CI the picker can stay on PUPickerUnavailableView, loading, when the simulator's photo service does not start.
        default: app.navigationBars["Photos"].appears(within: 30) ? app.navigationBars["Photos"] : app.navigationBars["PUPickerUnavailableView"]
        }
        // Cancel remote pickers before another presentation or terminating their host app.
        XCTAssertTrue(picker.appears(within: choice == "Saved prompts" ? 5 : 30), app.debugDescription)
        let cancel = picker.buttons["Cancel"]
        XCTAssertTrue(cancel.appears(within: 5), app.debugDescription)
        cancel.tap()
        XCTAssertTrue(picker.disappears(within: 5), app.debugDescription)
    }
}
