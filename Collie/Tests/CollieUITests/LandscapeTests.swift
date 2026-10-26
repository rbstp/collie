import XCTest

@MainActor
final class LandscapeTests: XCTestCase {
    override func tearDown() async throws {
        XCUIDevice.shared.orientation = .portrait
    }

    func testAgentsLayoutsAndUsage() throws {
        let app = try launchDemo()
        defer { app.terminate() }
        XCTAssertTrue(app.rotate(to: .landscapeLeft), app.debugDescription)
        let menu = Match(.button, "Layout")
        for (layout, row) in [("Grid", "fix the build"), ("List", "fix the build"), ("Inbox", "I need to run cargo test to check the new cases.")] {
            XCTAssertTrue(shown([menu], app), app.debugDescription)
            app.choose(app.buttons[layout]) { app.element(menu).tap() }
            let element = Match(.any, NSPredicate(format: "label CONTAINS %@", row))
            XCTAssertTrue(shown([element, Match(.button, "New task")], app), "\(layout)\n\(app.debugDescription)")
        }
        let view = app.segmentedControls["View"]
        view.buttons["Usage"].tap()
        let usage = Match(.staticText, "5-hour usage limit")
        XCTAssertTrue(app.element(usage).appears(within: 10), app.debugDescription)
        for orientation: UIDeviceOrientation in [.landscapeRight, .portrait, .landscapeLeft] {
            XCTAssertTrue(app.rotate(to: orientation), app.debugDescription)
            XCTAssertTrue(shown([usage], app), app.debugDescription)
            XCTAssertTrue(view.buttons["Usage"].isSelected, app.debugDescription)
        }
        app.buttons["Machines"].tap()
        XCTAssertTrue(shown([Match(.staticText, "MacBook Pro")], app), app.debugDescription)
    }

    func testSessionKeepsDraftFocusAndLatestOutputAcrossRotation() throws {
        let app = try launchDemo()
        defer { app.terminate() }
        app.open(try XCTUnwrap(URL(string: "collie://agent?m=demo&t=term_demo")))
        let terminalView = app.buttons["Terminal"].firstMatch
        XCTAssertTrue(terminalView.appears(within: 10), app.debugDescription)
        terminalView.tap()
        // The keyboard's candidate bar is a scroll view too, in its own window.
        let terminal = Match(.scrollView, NSPredicate(value: true))
        let portraitWidth = try XCTUnwrap(app.element(terminal).settledFrame(), app.debugDescription).width
        XCTAssertTrue(app.rotate(to: .landscapeLeft), app.debugDescription)
        XCTAssertTrue(eventually(within: 30) { app.element(terminal).frame.width > portraitWidth * 1.5 }, app.debugDescription)
        let keys = Match(.button, "Keys")
        XCTAssertTrue(shown([keys], app), app.debugDescription)
        if !app.element(keys).isSelected { app.element(keys).tap() }
        let field = Match(.textField, NSPredicate(value: true))
        XCTAssertTrue(shown([field], app), app.debugDescription)
        app.element(field).tap()
        let keyboard = app.keyboards.firstMatch
        XCTAssertTrue(keyboard.appears(within: 10), app.debugDescription)
        let draft = app.type("keep this draft", into: app.element(field))
        let escape = Match(.button, "Escape")
        let header = Match(.button, "Terminal")
        let controls = [
            Match(.any, NSPredicate(format: "value == %@", "keep this draft")), Match(.button, "Send"), Match(.button, "Hide keyboard"),
            Match(.button, "Attachments and saved prompts"), Match(.button, NSPredicate(format: "label BEGINSWITH %@", "Dictate in")),
            escape, Match(.button, "Control Return")
        ]
        for orientation: UIDeviceOrientation in [.landscapeLeft, .portrait, .landscapeRight] {
            XCTAssertTrue(app.rotate(to: orientation), "\(orientation.rawValue)\n\(app.debugDescription)")
            XCTAssertTrue(keyboard.appears(within: 30), "\(orientation.rawValue)\n\(app.debugDescription)")
            XCTAssertTrue(shown(controls, app) { screen, frames in
                guard let escape = screen[escape]?.frame, let top = screen[terminal]?.frame.minY else { return false }
                return aboveKeyboard(frames, app) && escape.minY - max(top, screen[header]?.frame.maxY ?? 0) > 24
            }, "\(orientation.rawValue)\n\(app.debugDescription)")
        }
        app.withInteractionOptions(2) { app.buttons["Hide keyboard"].tap() }
        XCTAssertTrue(keyboard.disappears(within: 10), app.debugDescription)
        XCTAssertTrue(draft.exists, app.debugDescription)
        app.withInteractionOptions(2) { app.buttons["Chat"].firstMatch.tap() }
        let latest = Match(.staticText, NSPredicate(format: "label BEGINSWITH %@", "Round 20:"))
        XCTAssertTrue(shown([latest], app), app.debugDescription)
        for orientation: UIDeviceOrientation in [.portrait, .landscapeLeft] {
            XCTAssertTrue(app.rotate(to: orientation), "\(orientation.rawValue)\n\(app.debugDescription)")
            XCTAssertTrue(shown([latest], app), "\(orientation.rawValue)\n\(app.debugDescription)")
        }
        XCTAssertTrue(draft.exists, app.debugDescription)
    }

    func testSheetsKeepPromptAcrossRotation() throws {
        let app = try launchDemo()
        defer { app.terminate() }
        app.buttons["New task"].tap()
        let prompt = app.textFields["What should the agent do?"]
        XCTAssertTrue(prompt.appears(within: 10), app.debugDescription)
        for _ in 0..<5 where !prompt.isHittable { app.collectionViews.firstMatch.swipeUp() }
        prompt.tap()
        let keyboard = app.keyboards.firstMatch
        XCTAssertTrue(keyboard.appears(within: 10), app.debugDescription)
        let typed = app.type("keep this prompt", into: prompt)
        let attachment = Match(.button, "Attachments and saved prompts")
        let controls = [Match(.any, NSPredicate(format: "value == %@", "keep this prompt")), Match(.button, "Hide keyboard"), attachment]
        for orientation: UIDeviceOrientation in [.portrait, .landscapeLeft, .portrait, .landscapeRight, .portrait] {
            XCTAssertTrue(app.rotate(to: orientation), "\(orientation.rawValue)\n\(app.debugDescription)")
            XCTAssertTrue(app.navigationBars["New task"].exists, app.debugDescription)
            XCTAssertTrue(keyboard.appears(within: 30), "\(orientation.rawValue)\n\(app.debugDescription)")
            XCTAssertTrue(shown(controls, app) { _, frames in aboveKeyboard(frames, app) }, "\(orientation.rawValue)\n\(app.debugDescription)")
        }
        app.choose(app.buttons["Saved prompts"].firstMatch) { app.element(attachment).tap() }
        let saved = app.navigationBars["Saved prompts"]
        XCTAssertTrue(saved.appears(within: 10), app.debugDescription)
        XCTAssertTrue(app.rotate(to: .portrait), app.debugDescription)
        XCTAssertTrue(saved.exists, app.debugDescription)
        let cancel = saved.buttons["Cancel"]
        XCTAssertNotNil(cancel.settledFrame(within: 30), app.debugDescription)
        XCTAssertTrue(eventually { cancel.isHittable }, app.debugDescription)
        cancel.tap()
        XCTAssertTrue(saved.disappears(within: 10), app.debugDescription)
        XCTAssertTrue(typed.exists, app.debugDescription)
    }

    func testApprovalCardLeavesTheComposerReachable() throws {
        let app = try launchDemo(["--approval-demo"])
        defer { app.terminate() }
        app.open(try XCTUnwrap(URL(string: "collie://agent?m=demo&t=term_2")))
        let approve = Match(.button, "Approve")
        XCTAssertTrue(app.element(approve).appears(within: 10), app.debugDescription)
        // Chat's scroll view takes accessibility hit tests under the header and the band.
        app.buttons["Terminal"].firstMatch.tap()
        XCTAssertTrue(app.rotate(to: .landscapeLeft), app.debugDescription)
        let field = Match(.textField, "Type an answer")
        let send = Match(.button, "Send answer")
        XCTAssertTrue(shown([field, send], app), app.debugDescription)
        XCTAssertTrue(shown([approve], app), app.debugDescription)
        let header = Match(.button, NSPredicate(format: "label BEGINSWITH %@", "Claude Code, "))
        let prompt = app.scrollViews.containing(NSPredicate(format: "label BEGINSWITH %@", "Claude Code, ")).firstMatch
        let command = Match(.staticText, NSPredicate(format: "label BEGINSWITH %@", "Bash: "))
        for _ in 0..<5 where !shown([command], app, within: 1) { drag(prompt, by: -20) }
        XCTAssertTrue(shown([command], app), app.debugDescription)
        for _ in 0..<5 where !shown([header], app, within: 1) { drag(prompt, by: 30) }
        app.element(field).tap()
        let keyboard = app.keyboards.firstMatch
        XCTAssertTrue(keyboard.appears(within: 10), app.debugDescription)
        XCTAssertTrue(shown([field, send], app) { _, frames in aboveKeyboard(frames, app) }, app.debugDescription)
        // The keyboard's own frame starts below its top strip: the card's header and the composer
        // field being hittable is what shows nothing went under it.
        XCTAssertTrue(shown([header], app), app.debugDescription)
        let deny = Match(.button, "Deny")
        for _ in 0..<8 where !shown([approve, deny], app, within: 1) { drag(prompt, by: -30) }
        XCTAssertTrue(shown([approve, deny], app), app.debugDescription)
        XCTAssertTrue(shown([field], app), app.debugDescription)
    }

    func testApprovalCardKeepsApproveInReachAtLargeText() throws {
        let app = try launchDemo(["--approval-demo", "-UIPreferredContentSizeCategoryName", "UICTContentSizeCategoryAccessibilityXXL"])
        defer { app.terminate() }
        app.open(try XCTUnwrap(URL(string: "collie://agent?m=demo&t=term_2")))
        let approve = app.buttons["Approve"]
        XCTAssertTrue(approve.appears(within: 10), app.debugDescription)
        // Chat's scroll view takes accessibility hit tests under the header and the band.
        app.buttons["Terminal"].firstMatch.tap()
        // The band caps its text below the accessibility sizes, so Approve stays pinned under the prompt. Not `shown`:
        // at this size the header and the band run a few points past the window's sides.
        XCTAssertTrue(eventually(within: 30) { approve.isHittable }, app.debugDescription)
        XCTAssertTrue(shown([Match(.textField, "Type an answer")], app), app.debugDescription)
    }

    /// A press and drag: XCUITest's swipes left the approval card unscrolled in landscape.
    private func drag(_ element: XCUIElement, by dy: CGFloat) {
        let start = element.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5))
        start.press(forDuration: 0.05, thenDragTo: start.withOffset(CGVector(dx: 0, dy: dy)))
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

    /// On screen: the same frame inside the main window on two polls in a row, accepted by `check`, then hittable.
    /// Frames lag the rotation, and asking for hittability mid-rotation fails the test.
    private func shown(
        _ matches: [Match], _ app: XCUIApplication, within timeout: TimeInterval = 30,
        where check: (Screen, [CGRect]) -> Bool = { _, _ in true }
    ) -> Bool {
        var last: [CGRect] = []
        return eventually(within: timeout) {
            guard let screen = app.screen() else { return false }
            let frames = matches.map { screen[$0]?.frame ?? .null }
            defer { last = frames }
            guard frames == last, frames.allSatisfy({ !$0.isEmpty && screen.window.contains($0) }), check(screen, frames) else { return false }
            // The query can resolve to another element than the snapshot did, and asking a frameless or off-window
            // element for hittability fails the test.
            return matches.allSatisfy { match in
                let element = app.element(match)
                let frame = element.frame
                return !frame.isEmpty && screen.window.contains(frame) && element.isHittable
            }
        }
    }

    private func aboveKeyboard(_ frames: [CGRect], _ app: XCUIApplication) -> Bool {
        let keyboard = app.keyboards.firstMatch
        guard keyboard.exists else { return false }
        let top = keyboard.frame.minY
        return frames.allSatisfy { $0.maxY <= top + 1 }
    }
}
