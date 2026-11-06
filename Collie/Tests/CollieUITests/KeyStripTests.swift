import XCTest

@MainActor
final class KeyStripTests: XCTestCase {
    func testToggleModifiersAndShortcuts() throws {
        continueAfterFailure = false
        let app = try demoApp("landscape-demo")
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
        XCTAssertTrue(app.buttons["Shift Tab"].firstMatch.exists, app.debugDescription)
        XCTAssertGreaterThanOrEqual(app.buttons["Shift Tab"].firstMatch.frame.height, 44, app.debugDescription)
        XCTAssertFalse(app.buttons["Shift Left arrow"].exists, app.debugDescription)
        // Shortcuts are kept on the simulator, so an earlier failed run may have left some.
        removeShortcuts(app)

        let shift = app.buttons["Shift"]
        shift.tap()
        XCTAssertTrue(shift.isSelected, app.debugDescription)
        XCTAssertFalse(app.buttons["Backspace"].isEnabled, app.debugDescription)
        XCTAssertTrue(app.buttons["Tab"].isEnabled, app.debugDescription)
        app.buttons["Tab"].tap()
        XCTAssertFalse(shift.isSelected, app.debugDescription)
        XCTAssertTrue(app.buttons["Backspace"].isEnabled, app.debugDescription)

        // The terminal view has no prompt field: the keyboard types straight into the terminal.
        XCTAssertFalse(app.textFields.firstMatch.exists, app.debugDescription)
        app.buttons["Keyboard"].tap()
        let keyboard = app.keyboards.firstMatch
        XCTAssertTrue(keyboard.appears(within: 5), app.debugDescription)
        let control = app.buttons["Control"]
        control.tap()
        XCTAssertTrue(control.isSelected, app.debugDescription)
        app.typeText("c")
        XCTAssertFalse(control.isSelected, app.debugDescription)
        app.typeText("ok")
        XCTAssertTrue(app.buttons["Arrow joystick"].exists || app.otherElements["Arrow joystick"].exists, app.debugDescription)
        app.buttons["Hide keyboard"].tap()
        XCTAssertTrue(keyboard.disappears(within: 5), app.debugDescription)

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

        // The saved key shortcut follows the built-in Shift Tab chip and reads the same.
        let custom = app.buttons.matching(NSPredicate(format: "label == %@", "Shift Tab")).element(boundBy: 1)
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

        removeShortcuts(app)
        XCTAssertTrue(effort.disappears(within: 5), app.debugDescription)

        keys.tap()
        XCTAssertTrue(escape.disappears(within: 5), app.debugDescription)
        XCTAssertFalse(keys.isSelected, app.debugDescription)
    }

    func testKeysFitAtLargerText() throws {
        continueAfterFailure = false
        let app = try demoApp("landscape-demo", ["-UIPreferredContentSizeCategoryName", "UICTContentSizeCategoryAccessibilityL"])
        app.launch()
        defer { app.terminate() }
        XCTAssertTrue(app.buttons["New task"].appears(within: 10), app.debugDescription)
        app.open(try XCTUnwrap(URL(string: "collie://agent?m=demo&t=term_demo")))
        let terminalView = app.buttons["Terminal"].firstMatch
        XCTAssertTrue(terminalView.appears(within: 10), app.debugDescription)
        terminalView.tap()
        let keys = app.buttons["Keys"]
        XCTAssertTrue(keys.appears(within: 5), app.debugDescription)
        let escape = app.buttons["Escape"]
        if !escape.exists { keys.tap() }
        XCTAssertTrue(escape.appears(within: 5), app.debugDescription)

        let window = app.windows.firstMatch.frame
        for name in ["Shift", "Control", "Escape", "Tab", "Backspace", "Return", "Arrow joystick"] {
            let key = app.descendants(matching: .any)[name].firstMatch
            XCTAssertTrue(key.isHittable && window.contains(key.frame), app.debugDescription)
        }
        let joystick = app.descendants(matching: .any)["Arrow joystick"].firstMatch
        XCTAssertEqual(joystick.frame.midY, escape.frame.midY, accuracy: 2, app.debugDescription)
    }

    func testJoystickWithoutTheStrip() throws {
        continueAfterFailure = false
        let app = try demoApp("landscape-demo")
        app.launch()
        defer { app.terminate() }
        XCTAssertTrue(app.buttons["New task"].appears(within: 10), app.debugDescription)
        app.open(try XCTUnwrap(URL(string: "collie://agent?m=demo&t=term_demo")))
        let terminalView = app.buttons["Terminal"].firstMatch
        XCTAssertTrue(terminalView.appears(within: 10), app.debugDescription)
        terminalView.tap()

        let keys = app.buttons["Keys"]
        XCTAssertTrue(keys.appears(within: 5), app.debugDescription)
        let escape = app.buttons["Escape"]
        if keys.isSelected { keys.tap() }
        XCTAssertTrue(escape.disappears(within: 5), app.debugDescription)
        let joysticks = app.descendants(matching: .any).matching(NSPredicate(format: "label == %@", "Arrow joystick"))
        let joystick = joysticks.firstMatch
        XCTAssertTrue(joystick.appears(within: 5), app.debugDescription)
        XCTAssertEqual(joysticks.count, 1, app.debugDescription)
        XCTAssertTrue(joystick.isHittable, app.debugDescription)
        let keyboardButton = app.buttons["Keyboard"]
        XCTAssertEqual(joystick.frame.midY, keys.frame.midY, accuracy: 2, app.debugDescription)
        XCTAssertEqual(joystick.frame.height, keyboardButton.frame.height, accuracy: 1, app.debugDescription)
        XCTAssertLessThan(joystick.frame.maxX, keyboardButton.frame.minX, app.debugDescription)

        keyboardButton.tap()
        let keyboard = app.keyboards.firstMatch
        XCTAssertTrue(keyboard.appears(within: 5), app.debugDescription)
        XCTAssertTrue(joystick.isHittable, app.debugDescription)
        app.buttons["Hide keyboard"].tap()
        XCTAssertTrue(keyboard.disappears(within: 5), app.debugDescription)

        keys.tap()
        XCTAssertTrue(escape.appears(within: 5), app.debugDescription)
        XCTAssertEqual(joysticks.count, 1, app.debugDescription)
        XCTAssertEqual(joystick.frame.midY, escape.frame.midY, accuracy: 2, app.debugDescription)
        keys.tap()
        XCTAssertTrue(escape.disappears(within: 5), app.debugDescription)

        app.buttons["Chat"].firstMatch.tap()
        XCTAssertTrue(keys.disappears(within: 5), app.debugDescription)
        XCTAssertEqual(joysticks.count, 0, app.debugDescription)
    }

    func testReturnWithoutTheStrip() throws {
        continueAfterFailure = false
        let app = try demoApp("landscape-demo")
        app.launch()
        defer { app.terminate() }
        XCTAssertTrue(app.buttons["New task"].appears(within: 10), app.debugDescription)
        app.open(try XCTUnwrap(URL(string: "collie://agent?m=demo&t=term_demo")))
        let terminalView = app.buttons["Terminal"].firstMatch
        XCTAssertTrue(terminalView.appears(within: 10), app.debugDescription)
        terminalView.tap()

        let keys = app.buttons["Keys"]
        XCTAssertTrue(keys.appears(within: 5), app.debugDescription)
        let escape = app.buttons["Escape"]
        if keys.isSelected { keys.tap() }
        XCTAssertTrue(escape.disappears(within: 5), app.debugDescription)
        let returns = app.buttons.matching(NSPredicate(format: "label == %@", "Return"))
        let returnKey = returns.firstMatch
        XCTAssertTrue(returnKey.appears(within: 5), app.debugDescription)
        XCTAssertEqual(returns.count, 1, app.debugDescription)
        XCTAssertTrue(returnKey.isHittable && app.windows.firstMatch.frame.contains(returnKey.frame), app.debugDescription)
        let keyboardButton = app.buttons["Keyboard"]
        XCTAssertEqual(returnKey.frame.midY, keys.frame.midY, accuracy: 2, app.debugDescription)
        XCTAssertEqual(returnKey.frame.height, keyboardButton.frame.height, accuracy: 1, app.debugDescription)
        XCTAssertLessThan(keyboardButton.frame.maxX, returnKey.frame.minX, app.debugDescription)

        keyboardButton.tap()
        let keyboard = app.keyboards.firstMatch
        XCTAssertTrue(keyboard.appears(within: 5), app.debugDescription)
        let composerReturn = try XCTUnwrap(returns.allElementsBoundByIndex.first { !keyboard.frame.intersects($0.frame) }, app.debugDescription)
        XCTAssertTrue(composerReturn.isHittable, app.debugDescription)
        composerReturn.tap()
        XCTAssertTrue(keyboard.disappears(within: 5), app.debugDescription)

        keys.tap()
        XCTAssertTrue(escape.appears(within: 5), app.debugDescription)
        XCTAssertEqual(returns.count, 1, app.debugDescription)
        XCTAssertEqual(returnKey.frame.midY, escape.frame.midY, accuracy: 2, app.debugDescription)
        keys.tap()
        XCTAssertTrue(escape.disappears(within: 5), app.debugDescription)

        app.buttons["Chat"].firstMatch.tap()
        XCTAssertTrue(keys.disappears(within: 5), app.debugDescription)
        XCTAssertEqual(returns.count, 0, app.debugDescription)
    }

    func testReturnHidesTheTerminalKeyboard() throws {
        continueAfterFailure = false
        let app = try demoApp("landscape-demo")
        app.launch()
        defer { app.terminate() }
        XCTAssertTrue(app.buttons["New task"].appears(within: 10), app.debugDescription)
        app.open(try XCTUnwrap(URL(string: "collie://agent?m=demo&t=term_demo")))
        let terminalView = app.buttons["Terminal"].firstMatch
        XCTAssertTrue(terminalView.appears(within: 10), app.debugDescription)
        terminalView.tap()
        app.buttons["Keyboard"].tap()
        let keyboard = app.keyboards.firstMatch
        XCTAssertTrue(keyboard.appears(within: 5), app.debugDescription)
        app.typeText("ok")
        XCTAssertFalse(keyboard.disappears(within: 2), app.debugDescription)
        app.typeText("\n")
        XCTAssertTrue(keyboard.disappears(within: 5), app.debugDescription)
    }

    private func removeShortcuts(_ app: XCUIApplication) {
        app.buttons["Edit key shortcuts"].tap()
        XCTAssertTrue(app.buttons["Add shortcut"].appears(within: 10), app.debugDescription)
        let empty = app.staticTexts["No key shortcuts"]
        for _ in 0..<20 where !empty.exists {
            let row = app.cells.firstMatch
            row.swipeLeft()
            let delete = app.buttons["Delete"]
            XCTAssertTrue(delete.appears(within: 5), app.debugDescription)
            delete.tap()
        }
        XCTAssertTrue(empty.exists, app.debugDescription)
        app.buttons["Done"].tap()
    }
}
