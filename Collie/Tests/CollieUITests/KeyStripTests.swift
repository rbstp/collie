import XCTest

@MainActor
final class KeyStripTests: XCTestCase {
    func testToggleModifiersAndShortcuts() throws {
        continueAfterFailure = false
        let app = XCUIApplication()
        let fixture = try XCTUnwrap(Bundle(for: Self.self).url(forResource: "landscape-demo", withExtension: "txt"))
        app.launchArguments = ["-AppleLanguages", "(en)", "-AppleLocale", "en_US", "--terminal-demo", fixture.path]
        app.launch()
        defer { app.terminate() }
        XCTAssertTrue(app.buttons["New task"].appears(within: 10), app.debugDescription)
        app.open(try XCTUnwrap(URL(string: "collie://agent?m=demo&t=term_demo")))
        let terminalView = app.buttons["Terminal"].firstMatch
        XCTAssertTrue(terminalView.appears(within: 10), app.debugDescription)
        terminalView.tap()

        let keys = app.buttons["Keys"]
        XCTAssertTrue(keys.appears(within: 5), app.debugDescription)
        let attachment = app.buttons["Attachments and saved prompts"]
        XCTAssertLessThan(attachment.frame.maxX, keys.frame.minX, app.debugDescription)
        if keys.isSelected { keys.tap() }
        let escape = app.buttons["Escape"]
        XCTAssertTrue(escape.disappears(within: 5), app.debugDescription)
        keys.tap()
        XCTAssertTrue(escape.appears(within: 5), app.debugDescription)
        XCTAssertTrue(keys.isSelected, app.debugDescription)

        let shift = app.buttons["Shift"]
        shift.tap()
        XCTAssertTrue(shift.isSelected, app.debugDescription)
        XCTAssertFalse(app.buttons["Up arrow"].isEnabled, app.debugDescription)
        XCTAssertTrue(app.buttons["Tab"].isEnabled, app.debugDescription)
        app.buttons["Tab"].tap()
        XCTAssertFalse(shift.isSelected, app.debugDescription)
        XCTAssertTrue(app.buttons["Up arrow"].isEnabled, app.debugDescription)

        let field = app.textFields.firstMatch
        field.tap()
        XCTAssertTrue(app.keyboards.firstMatch.appears(within: 5), app.debugDescription)
        let clear = app.buttons["Clear text"]
        if clear.exists { clear.tap() }
        let control = app.buttons["Control"]
        control.tap()
        XCTAssertTrue(control.isSelected, app.debugDescription)
        app.typeText("c")
        XCTAssertFalse(control.isSelected, app.debugDescription)
        app.typeText("ok")
        let typed = app.textFields.matching(NSPredicate(format: "value == %@", "ok")).firstMatch
        XCTAssertTrue(typed.appears(within: 5), app.debugDescription)
        clear.tap()
        app.buttons["Hide keyboard"].tap()

        app.buttons["Edit key shortcuts"].tap()
        let addShortcut = app.buttons["Add shortcut"]
        XCTAssertTrue(addShortcut.appears(within: 5), app.debugDescription)
        addShortcut.tap()
        app.buttons["Save"].tap()
        addShortcut.tap()
        app.buttons["Text"].tap()
        let text = app.textFields["/effort high"]
        XCTAssertTrue(text.appears(within: 5), app.debugDescription)
        text.tap()
        text.typeText("/effort high")
        app.buttons["Save"].tap()
        app.buttons["Done"].tap()

        let custom = app.buttons.matching(NSPredicate(format: "label == %@", "⇧⇥")).firstMatch
        let effort = app.buttons["/effort high"]
        XCTAssertTrue(custom.appears(within: 5), app.debugDescription)
        XCTAssertTrue(effort.appears(within: 5), app.debugDescription)
        XCTAssertTrue(custom.isEnabled && effort.isEnabled, app.debugDescription)
        let screenshot = XCTAttachment(screenshot: app.screenshot())
        screenshot.name = "key-strip"
        screenshot.lifetime = .keepAlways
        add(screenshot)
        custom.tap()
        effort.tap()

        app.buttons["Edit key shortcuts"].tap()
        XCTAssertTrue(addShortcut.appears(within: 5), app.debugDescription)
        for label in ["/effort high", "⇧⇥"] {
            let row = app.cells.containing(NSPredicate(format: "label == %@", label)).firstMatch
            XCTAssertTrue(row.appears(within: 5), app.debugDescription)
            row.swipeLeft()
            app.buttons["Delete"].tap()
            XCTAssertTrue(row.disappears(within: 5), app.debugDescription)
        }
        app.buttons["Done"].tap()
        XCTAssertTrue(effort.disappears(within: 5), app.debugDescription)

        keys.tap()
        XCTAssertTrue(escape.disappears(within: 5), app.debugDescription)
        XCTAssertFalse(keys.isSelected, app.debugDescription)
    }
}
