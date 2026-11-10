import XCTest

@MainActor
final class KeyStripTests: XCTestCase {
    func testModifiersShortcutsAndTheBottomRowWithoutTheStrip() throws {
        continueAfterFailure = false
        let app = try demoApp("landscape-demo")
        try app.launch(onSession: "term_demo")
        defer { app.terminate() }
        let terminalView = app.buttons["Terminal"].firstMatch
        XCTAssertTrue(terminalView.appears(within: 10), app.debugDescription)
        // A terminal session can stop the app from idling, and XCTest then waited 60 s around a tap (see choose).
        app.withInteractionOptions(2) { terminalView.tap() }

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
        app.withInteractionOptions(2) { app.buttons["Keyboard"].tap() }
        let keyboard = app.keyboards.firstMatch
        XCTAssertTrue(keyboard.appears(within: 5), app.debugDescription)
        let control = app.buttons["Control"]
        control.tap()
        XCTAssertTrue(control.isSelected, app.debugDescription)
        app.typeText("c")
        XCTAssertFalse(control.isSelected, app.debugDescription)
        app.buttons["Hide keyboard"].tap()
        XCTAssertTrue(keyboard.disappears(within: 5), app.debugDescription)

        app.buttons["Edit key shortcuts"].tap()
        let addShortcut = app.buttons["Add shortcut"]
        XCTAssertTrue(addShortcut.appears(within: 5), app.debugDescription)
        // Shortcuts are kept on the simulator, so a failed run, or the failed first iteration before a retry, leaves its own.
        let empty = app.staticTexts["No key shortcuts"]
        let removeShortcuts = {
            for _ in 0..<20 {
                guard !empty.exists else { break }
                app.cells.firstMatch.swipeLeft()
                let delete = app.buttons["Delete"]
                XCTAssertTrue(delete.appears(within: 5), app.debugDescription)
                delete.tap()
            }
            XCTAssertTrue(empty.exists, app.debugDescription)
        }
        removeShortcuts()
        addShortcut.tap()
        let save = app.buttons["Save"]
        XCTAssertTrue(save.appears(within: 5), app.debugDescription)
        XCTAssertFalse(save.isEnabled, app.debugDescription)
        for name in ["Control", "Option"] {
            let modifier = app.switches.matching(NSPredicate(format: "label == %@", name)).firstMatch
            XCTAssertTrue(modifier.appears(within: 5), app.debugDescription)
            modifier.tap()
        }
        let key = app.textFields["Key"]
        key.tap()
        key.typeText("x")
        XCTAssertTrue(save.isEnabled, app.debugDescription)
        save.tap()
        XCTAssertTrue(app.staticTexts["⌃⌥X"].appears(within: 5), app.debugDescription)
        app.buttons["Done"].tap()

        // The saved key shortcut follows the built-in chips and reads its modifiers and key.
        let custom = app.buttons["Control Option X"]
        XCTAssertTrue(custom.appears(within: 5), app.debugDescription)
        XCTAssertTrue(custom.isEnabled, app.debugDescription)
        custom.tap()

        app.buttons["Edit key shortcuts"].tap()
        XCTAssertTrue(addShortcut.appears(within: 10), app.debugDescription)
        removeShortcuts()
        app.buttons["Done"].tap()
        XCTAssertTrue(custom.disappears(within: 5), app.debugDescription)

        keys.tap()
        XCTAssertTrue(escape.disappears(within: 5), app.debugDescription)
        XCTAssertFalse(keys.isSelected, app.debugDescription)

        // Without the strip, the composer keeps the joystick and a Return key.
        let joysticks = app.descendants(matching: .any).matching(NSPredicate(format: "label == %@", "Arrow joystick"))
        let returns = app.buttons.matching(NSPredicate(format: "label == %@", "Return"))
        let (joystick, returnKey, keyboardButton) = (joysticks.firstMatch, returns.firstMatch, app.buttons["Keyboard"])
        XCTAssertTrue(joystick.appears(within: 5), app.debugDescription)
        XCTAssertEqual(joysticks.count, 1, app.debugDescription)
        XCTAssertEqual(returns.count, 1, app.debugDescription)
        XCTAssertTrue(joystick.isHittable && returnKey.isHittable, app.debugDescription)
        // The bottom row: the joystick, the keyboard button and Return, level with Keys.
        let row = try XCTUnwrap(app.screen(), app.debugDescription)
        let frame = { (match: Match) in try XCTUnwrap(row[match]?.frame, app.debugDescription) }
        let (stick, button, enter) = (try frame(Match(.any, "Arrow joystick")), try frame(Match(.button, "Keyboard")), try frame(Match(.button, "Return")))
        let level = try frame(Match(.button, "Keys")).midY
        XCTAssertEqual(stick.midY, level, accuracy: 2, app.debugDescription)
        XCTAssertEqual(enter.midY, level, accuracy: 2, app.debugDescription)
        XCTAssertEqual(stick.height, button.height, accuracy: 1, app.debugDescription)
        XCTAssertEqual(enter.height, button.height, accuracy: 1, app.debugDescription)
        XCTAssertLessThan(stick.maxX, button.minX, app.debugDescription)
        XCTAssertLessThan(button.maxX, enter.minX, app.debugDescription)
        XCTAssertTrue(row.window.contains(enter), app.debugDescription)

        app.withInteractionOptions(2) { keyboardButton.tap() }
        XCTAssertTrue(keyboard.appears(within: 5), app.debugDescription)
        XCTAssertTrue(joystick.isHittable, app.debugDescription)
        app.typeText("ok")
        XCTAssertTrue(keyboard.exists, app.debugDescription)
        app.typeText("\n")
        XCTAssertTrue(keyboard.disappears(within: 5), app.debugDescription)
        app.withInteractionOptions(2) { keyboardButton.tap() }
        XCTAssertTrue(keyboard.appears(within: 5), app.debugDescription)
        let composerReturn = try XCTUnwrap(returns.allElementsBoundByIndex.first { !keyboard.frame.intersects($0.frame) }, app.debugDescription)
        XCTAssertTrue(composerReturn.isHittable, app.debugDescription)
        composerReturn.tap()
        XCTAssertTrue(keyboard.disappears(within: 5), app.debugDescription)

        keys.tap()
        XCTAssertTrue(escape.appears(within: 5), app.debugDescription)
        XCTAssertEqual(joysticks.count, 1, app.debugDescription)
        XCTAssertEqual(returns.count, 1, app.debugDescription)
        XCTAssertEqual(joystick.frame.midY, escape.frame.midY, accuracy: 2, app.debugDescription)
        XCTAssertEqual(returnKey.frame.midY, escape.frame.midY, accuracy: 2, app.debugDescription)
        keys.tap()
        XCTAssertTrue(escape.disappears(within: 5), app.debugDescription)

        app.buttons["Chat"].firstMatch.tap()
        XCTAssertTrue(keys.disappears(within: 5), app.debugDescription)
        XCTAssertEqual(joysticks.count, 0, app.debugDescription)
        XCTAssertEqual(returns.count, 0, app.debugDescription)
    }
}
