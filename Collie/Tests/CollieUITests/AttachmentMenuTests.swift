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
            app.open(try XCTUnwrap(URL(string: "collie://agent?m=demo&t=term_demo")))
        }
        let placeholder = newTask ? "What should the agent do?" : "Prompt the agent"
        let field = app.textFields.matching(NSPredicate(format: "placeholderValue == %@ OR label == %@ OR value == %@", placeholder, placeholder, "test")).firstMatch
        // The slow system pickers are presented over the keyboard only.
        for keyboard in [false, true] {
            for choice in keyboard ? ["Saved prompts", "Files", "Photo Library"] : ["Saved prompts"] {
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
        app.choose(app.buttons[choice].firstMatch) { attachment.tap() }
        let (picker, timeout): (XCUIElement, TimeInterval) = switch choice {
        case "Saved prompts": (app.navigationBars["Saved prompts"], 5)
        // The document browser service starts with each launch. On CI its first presentation takes 9 to 28 s and sometimes
        // over 30 s, with the sheet up but empty until the service draws, so there is no loading view to bind to.
        case "Files": (app.navigationBars["FullDocumentManagerViewControllerNavigationBar"], 60)
        // On CI the picker can stay on PUPickerUnavailableView, loading, for over 30 s, then become Photos mid-cancel.
        default: (app.navigationBars.matching(NSPredicate(format: "identifier IN %@", ["Photos", "PUPickerUnavailableView"])).firstMatch, 30)
        }
        if choice == "Photo Library" {
            _ = app.navigationBars["Photos"].appears(within: 30)
        }
        // Cancel remote pickers before another presentation or terminating their host app.
        XCTAssertTrue(picker.appears(within: timeout), app.debugDescription)
        let cancel = picker.buttons.matching(identifier: "Cancel")
        // While Photos replaces the loading view, its Cancel can be unhittable or drop the tap.
        for _ in 0..<(choice == "Photo Library" ? 2 : 1) where picker.exists {
            XCTAssertTrue(cancel.firstMatch.appears(within: 5), app.debugDescription)
            cancel.allElementsBoundByIndex.first(where: \.isHittable)?.tap()
            _ = picker.disappears(within: 5)
        }
        XCTAssertFalse(picker.exists, app.debugDescription)
    }
}
