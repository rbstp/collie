import XCTest

@MainActor
final class LandscapeTests: XCTestCase {
    override func tearDown() async throws {
        XCUIDevice.shared.orientation = .portrait
    }

    func testAgentsLayoutsAndUsage() throws {
        let app = try launchDemo()
        defer { app.terminate() }
        rotate(.landscapeLeft, app)
        for (layout, row) in [("Grid", "fix the build"), ("List", "fix the build"), ("Inbox", "I need to run cargo test to check the new cases.")] {
            let menu = app.buttons["Layout"]
            XCTAssertTrue(eventually { shown(menu, app) }, app.debugDescription)
            menu.tap()
            app.buttons[layout].tap()
            let element = app.descendants(matching: .any).matching(NSPredicate(format: "label CONTAINS %@", row)).firstMatch
            XCTAssertTrue(eventually { shown(element, app) }, "\(layout)\n\(app.debugDescription)")
            XCTAssertTrue(app.buttons["New task"].isHittable, app.debugDescription)
        }
        let view = app.segmentedControls["View"]
        view.buttons["Usage"].tap()
        let usage = app.staticTexts["5-hour usage limit"]
        XCTAssertTrue(usage.appears(within: 5), app.debugDescription)
        for orientation: UIDeviceOrientation in [.landscapeRight, .portrait, .landscapeLeft] {
            rotate(orientation, app)
            XCTAssertTrue(eventually { shown(usage, app) }, app.debugDescription)
            XCTAssertTrue(view.buttons["Usage"].isSelected, app.debugDescription)
        }
        app.buttons["Machines"].tap()
        let machine = app.staticTexts["MacBook Pro"]
        XCTAssertTrue(eventually { shown(machine, app) }, app.debugDescription)
    }

    func testSessionKeepsDraftFocusAndLatestOutputAcrossRotation() throws {
        let app = try launchDemo()
        defer { app.terminate() }
        app.open(try XCTUnwrap(URL(string: "collie://agent?m=demo&t=term_demo")))
        let terminalView = app.buttons["Terminal"].firstMatch
        XCTAssertTrue(terminalView.appears(within: 10), app.debugDescription)
        terminalView.tap()
        let terminal = app.scrollViews.firstMatch
        XCTAssertTrue(terminal.appears(within: 5), app.debugDescription)
        let portraitWidth = terminal.frame.width
        rotate(.landscapeLeft, app)
        XCTAssertTrue(eventually { terminal.frame.width > portraitWidth * 1.5 }, app.debugDescription)
        let keys = app.buttons["Keys"]
        XCTAssertTrue(eventually { shown(keys, app) }, app.debugDescription)
        if !keys.isSelected { keys.tap() }
        let field = app.textFields.firstMatch
        XCTAssertTrue(eventually { shown(field, app) }, app.debugDescription)
        field.tap()
        let keyboard = app.keyboards.firstMatch
        XCTAssertTrue(keyboard.appears(within: 5), app.debugDescription)
        let draft = typeAfterClearing("keep this draft", into: field, app)
        let dictation = app.buttons.matching(NSPredicate(format: "label BEGINSWITH %@", "Dictate in")).firstMatch
        let controls = [
            draft, app.buttons["Send"], app.buttons["Hide keyboard"], app.buttons["Attachments and saved prompts"], dictation,
            app.buttons["Escape"], app.buttons["Control Return"]
        ]
        for orientation: UIDeviceOrientation in [.landscapeLeft, .portrait, .landscapeRight] {
            rotate(orientation, app)
            XCTAssertTrue(keyboard.appears(within: 5), "\(orientation.rawValue)\n\(app.debugDescription)")
            XCTAssertTrue(eventually {
                controls.allSatisfy { shown($0, app) && $0.frame.maxY <= keyboard.frame.minY + 1 }
                    && terminal.frame.height > 40 && terminal.frame.maxY <= keyboard.frame.minY + 1
            }, "\(orientation.rawValue)\n\(app.debugDescription)")
        }
        app.buttons["Hide keyboard"].tap()
        XCTAssertTrue(keyboard.disappears(within: 5), app.debugDescription)
        XCTAssertTrue(draft.exists, app.debugDescription)
        app.buttons["Chat"].firstMatch.tap()
        let latest = app.staticTexts.matching(NSPredicate(format: "label BEGINSWITH %@", "Round 20:")).firstMatch
        XCTAssertTrue(eventually { shown(latest, app) }, app.debugDescription)
        for orientation: UIDeviceOrientation in [.portrait, .landscapeLeft] {
            rotate(orientation, app)
            XCTAssertTrue(eventually { shown(latest, app) }, "\(orientation.rawValue)\n\(app.debugDescription)")
        }
        XCTAssertTrue(draft.exists, app.debugDescription)
    }

    func testSheetsKeepPromptAcrossRotation() throws {
        let app = try launchDemo()
        defer { app.terminate() }
        app.buttons["New task"].tap()
        let prompt = app.textFields["What should the agent do?"]
        XCTAssertTrue(prompt.appears(within: 5), app.debugDescription)
        for _ in 0..<5 where !prompt.isHittable { app.collectionViews.firstMatch.swipeUp() }
        prompt.tap()
        let keyboard = app.keyboards.firstMatch
        XCTAssertTrue(keyboard.appears(within: 5), app.debugDescription)
        let typed = typeAfterClearing("keep this prompt", into: prompt, app)
        let controls = [typed, app.buttons["Hide keyboard"], app.buttons["Attachments and saved prompts"]]
        for orientation: UIDeviceOrientation in [.landscapeLeft, .portrait, .landscapeRight] {
            rotate(orientation, app)
            XCTAssertTrue(app.navigationBars["New task"].exists, app.debugDescription)
            XCTAssertTrue(keyboard.appears(within: 5), "\(orientation.rawValue)\n\(app.debugDescription)")
            XCTAssertTrue(eventually {
                controls.allSatisfy { shown($0, app) && $0.frame.maxY <= keyboard.frame.minY + 1 }
            }, "\(orientation.rawValue)\n\(app.debugDescription)")
        }
        app.buttons["Attachments and saved prompts"].tap()
        let choice = app.buttons["Saved prompts"].firstMatch
        XCTAssertTrue(choice.appears(within: 5), app.debugDescription)
        choice.tap()
        let saved = app.navigationBars["Saved prompts"]
        XCTAssertTrue(saved.appears(within: 5), app.debugDescription)
        rotate(.portrait, app)
        XCTAssertTrue(saved.exists, app.debugDescription)
        let cancel = saved.buttons["Cancel"]
        XCTAssertTrue(eventually { shown(cancel, app) }, app.debugDescription)
        cancel.tap()
        XCTAssertTrue(saved.disappears(within: 5), app.debugDescription)
        XCTAssertTrue(typed.exists, app.debugDescription)
    }

    func testApprovalCardLeavesTheComposerReachable() throws {
        let app = try launchDemo(["--approval-demo"])
        defer { app.terminate() }
        app.open(try XCTUnwrap(URL(string: "collie://agent?m=demo&t=term_2")))
        let approve = app.buttons["Approve"]
        XCTAssertTrue(approve.appears(within: 10), app.debugDescription)
        rotate(.landscapeLeft, app)
        let field = app.textFields["Type an answer"]
        let send = app.buttons["Send answer"]
        XCTAssertTrue(eventually { shown(field, app) && shown(send, app) }, app.debugDescription)
        let cards = app.scrollViews.containing(.button, identifier: "Approve").firstMatch
        for _ in 0..<5 where !shown(approve, app) { cards.swipeUp() }
        XCTAssertTrue(shown(approve, app), app.debugDescription)
        field.tap()
        let keyboard = app.keyboards.firstMatch
        XCTAssertTrue(keyboard.appears(within: 5), app.debugDescription)
        XCTAssertTrue(eventually {
            [field, send].allSatisfy { shown($0, app) && $0.frame.maxY <= keyboard.frame.minY + 1 }
        }, app.debugDescription)
    }

    private func launchDemo(_ arguments: [String] = []) throws -> XCUIApplication {
        continueAfterFailure = false
        let app = XCUIApplication()
        let fixture = try XCTUnwrap(Bundle(for: Self.self).url(forResource: "landscape-demo", withExtension: "txt"))
        app.launchArguments = ["-AppleLanguages", "(en)", "-AppleLocale", "en_US", "--terminal-demo", fixture.path] + arguments
        app.launch()
        XCTAssertTrue(app.buttons["New task"].appears(within: 10), app.debugDescription)
        return app
    }

    /// Clears a draft an earlier run left, and types again when the simulator drops keystrokes.
    private func typeAfterClearing(_ text: String, into field: XCUIElement, _ app: XCUIApplication) -> XCUIElement {
        let clear = app.buttons["Clear text"]
        if clear.exists { clear.tap() }
        field.typeText(text)
        let typed = app.textFields.matching(NSPredicate(format: "value == %@", text)).firstMatch
        for _ in 0..<3 where !typed.appears(within: 10) {
            clear.tap()
            field.typeText(text)
        }
        XCTAssertTrue(typed.appears(within: 10), app.debugDescription)
        return typed
    }

    // Element frames lag the rotation, so callers wait on what they check next.
    private func rotate(_ orientation: UIDeviceOrientation, _ app: XCUIApplication) {
        XCUIDevice.shared.orientation = orientation
        let window = app.windows.firstMatch
        XCTAssertTrue(eventually { (window.frame.width > window.frame.height) == orientation.isLandscape }, app.debugDescription)
    }

    /// On screen once its frame stops moving; asking for hittability mid-rotation fails the test.
    private func shown(_ element: XCUIElement, _ app: XCUIApplication) -> Bool {
        guard element.exists else { return false }
        let frame = element.frame
        usleep(300_000)
        return element.frame == frame && app.windows.firstMatch.frame.contains(frame) && element.isHittable
    }

    private func eventually(within timeout: TimeInterval = 30, _ condition: () -> Bool) -> Bool {
        let deadline = Date.now.addingTimeInterval(timeout)
        while !condition() {
            if Date.now > deadline { return false }
            usleep(250_000)
        }
        return true
    }
}
