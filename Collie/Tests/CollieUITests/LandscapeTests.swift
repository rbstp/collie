import XCTest

@MainActor
final class LandscapeTests: XCTestCase {
    private let command = "cargo test -p collied --test approvals -- --nocapture"

    // An async tearDown makes XCTest end the runner on a failure when continueAfterFailure is false, and xcodebuild then
    // skips -retry-tests-on-failure for the test.
    override func tearDown() {
        MainActor.assumeIsolated { XCUIDevice.shared.orientation = .portrait }
    }

    func testAgentsLayoutsUsageAndANewTaskPromptAcrossRotation() throws {
        continueAfterFailure = false
        let app = try demoApp("landscape-demo")
        app.launch()
        defer { app.terminate() }
        XCTAssertTrue(app.buttons["New task"].appears(within: 10), app.debugDescription)
        XCTAssertTrue(app.rotate(to: .landscapeLeft), app.debugDescription)
        let menu = Match(.button, "Layout")
        for (layout, row) in [("Inbox", "I need to run cargo test to check the new cases."), ("Grid", "fix the build"), ("List", "fix the build")] {
            XCTAssertTrue(shown([menu], app), app.debugDescription)
            app.choose(app.buttons[layout]) { app.element(menu).tap() }
            let element = Match(.any, NSPredicate(format: "label CONTAINS %@", row))
            XCTAssertTrue(shown([element, Match(.button, "New task")], app), "\(layout)\n\(app.debugDescription)")
            if layout != "Inbox" {
                // With one machine paired, its header keeps the approvals pill and leaves the name out.
                XCTAssertTrue(app.descendants(matching: .any)["1 approval"].exists, "\(layout)\n\(app.debugDescription)")
                XCTAssertFalse(app.staticTexts["MacBook Pro"].exists, "\(layout)\n\(app.debugDescription)")
            }
        }
        let view = app.segmentedControls["View"]
        // The lists' cells, not the pager's pages.
        let rows = app.collectionViews.firstMatch.collectionViews.cells
        let row = rows.containing(NSPredicate(format: "label CONTAINS %@", "fix the build")).firstMatch
        let card = rows.containing(NSPredicate(format: "label == %@", "5-hour limit")).firstMatch
        view.buttons["Usage"].tap()
        let usage = Match(.staticText, "5-hour limit")
        // Usage stays selected across a rotation, and its cards keep the Agents list's side margins in both orientations.
        for orientation: UIDeviceOrientation in [.landscapeLeft, .portrait] {
            XCTAssertTrue(app.rotate(to: orientation), app.debugDescription)
            XCTAssertTrue(shown([usage], app), app.debugDescription)
            XCTAssertTrue(view.buttons["Usage"].isSelected, app.debugDescription)
            let cardFrame = try XCTUnwrap(card.settledFrame(), app.debugDescription)
            view.buttons["Agents"].tap()
            let rowFrame = try XCTUnwrap(row.settledFrame(), app.debugDescription)
            XCTAssertEqual(cardFrame.minX, rowFrame.minX, "\(orientation.rawValue)\n\(app.debugDescription)")
            XCTAssertEqual(cardFrame.maxX, rowFrame.maxX, "\(orientation.rawValue)\n\(app.debugDescription)")
            if orientation.isLandscape { view.buttons["Usage"].tap() }
        }

        let newTask = app.buttons["New task"]
        newTask.tap()
        let prompt = app.textFields["What should the agent do?"]
        XCTAssertTrue(prompt.appears(within: 10), app.debugDescription)
        prompt.tap()
        let keyboard = app.keyboards.firstMatch
        XCTAssertTrue(keyboard.appears(within: 10), app.debugDescription)
        let typed = app.type("keep this prompt", into: prompt)
        let attachment = Match(.button, "Attachments and saved prompts")
        let controls = [Match(.any, NSPredicate(format: "value == %@", "keep this prompt")), Match(.button, "Hide keyboard"), attachment]
        // The second landscape pass is the one that left the prompt row under the navigation bar.
        for orientation: UIDeviceOrientation in [.landscapeLeft, .portrait, .landscapeRight, .portrait] {
            XCTAssertTrue(app.rotate(to: orientation), "\(orientation.rawValue)\n\(app.debugDescription)")
            XCTAssertTrue(app.navigationBars["New task"].exists, app.debugDescription)
            XCTAssertTrue(keyboard.appears(within: 30), "\(orientation.rawValue)\n\(app.debugDescription)")
            XCTAssertTrue(shown(controls, app) { _, frames in aboveKeyboard(frames, app) }, "\(orientation.rawValue)\n\(app.debugDescription)")
        }
        app.choose(app.buttons["Saved prompts"].firstMatch) { app.element(attachment).tap() }
        let saved = app.navigationBars["Saved prompts"]
        XCTAssertTrue(saved.appears(within: 10), app.debugDescription)
        saved.buttons["Cancel"].tap()
        XCTAssertTrue(saved.disappears(within: 10), app.debugDescription)
        XCTAssertTrue(typed.exists, app.debugDescription)
    }

    func testChatOpensAFileAndScrollsBackAndTheTerminalKeepsTheKeyboardAcrossRotation() throws {
        continueAfterFailure = false
        let app = try demoApp("landscape-demo")
        try app.launch(onSession: "term_demo")
        defer { app.terminate() }
        let round20 = app.staticTexts.matching(NSPredicate(format: "label BEGINSWITH %@", "Round 20:")).firstMatch
        XCTAssertTrue(round20.appears(within: 10), app.debugDescription)
        // The approval band test checks that the header leaves the elapsed time out while an approval shows.
        XCTAssertTrue(app.elapsedTimes.firstMatch.exists, app.debugDescription)

        let link = app.links["tests/approvals.rs"]
        XCTAssertTrue(link.exists, app.debugDescription)
        link.tap()
        let content = app.staticTexts.matching(NSPredicate(format: "label BEGINSWITH %@", "// Sample of approvals.rs")).firstMatch
        XCTAssertTrue(content.appears(within: 10), app.debugDescription)
        XCTAssertTrue(app.buttons["Copy"].exists, app.debugDescription)
        app.buttons["Done"].tap()
        XCTAssertTrue(content.disappears(within: 5), app.debugDescription)

        let button = app.buttons["Latest messages"]
        XCTAssertFalse(button.exists, app.debugDescription)
        let start = app.windows.firstMatch.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.4))
        start.press(forDuration: 0.1, thenDragTo: start.withOffset(CGVector(dx: 0, dy: 250)))
        XCTAssertTrue(button.appears(within: 5), app.debugDescription)
        button.tap()
        XCTAssertTrue(button.disappears(within: 5), app.debugDescription)

        let wake = app.buttons.matching(NSPredicate(format: "label CONTAINS %@", "PR watch: CI failed on #264")).firstMatch
        XCTAssertTrue(wake.appears(within: 5), app.debugDescription)
        let wakePrompt = app.staticTexts.matching(NSPredicate(format: "label BEGINSWITH %@", "Pull request #264")).firstMatch
        XCTAssertFalse(wakePrompt.exists, app.debugDescription)
        wake.tap()
        XCTAssertTrue(wakePrompt.appears(within: 5), app.debugDescription)

        let terminalView = app.buttons["Terminal"].firstMatch
        terminalView.tap()
        // The keyboard's candidate bar is a scroll view too, in its own window.
        let terminal = Match(.scrollView, NSPredicate(value: true))
        let portraitWidth = try XCTUnwrap(app.element(terminal).settledFrame(), app.debugDescription).width
        XCTAssertTrue(app.rotate(to: .landscapeLeft), app.debugDescription)
        XCTAssertTrue(eventually(within: 30) { app.element(terminal).frame.width > portraitWidth * 1.5 }, app.debugDescription)
        let keys = Match(.button, "Keys")
        XCTAssertTrue(shown([keys], app), app.debugDescription)
        if !app.element(keys).isSelected { app.element(keys).tap() }
        let showKeyboard = Match(.button, "Keyboard")
        XCTAssertTrue(shown([showKeyboard], app), app.debugDescription)
        app.withInteractionOptions(2) { app.element(showKeyboard).tap() }
        let keyboard = app.keyboards.firstMatch
        XCTAssertTrue(keyboard.appears(within: 10), app.debugDescription)
        let escape = Match(.button, "Escape")
        let header = Match(.button, "Terminal")
        let controls = [
            Match(.button, "Hide keyboard"), Match(.button, "Attachments and saved prompts"),
            Match(.button, NSPredicate(format: "label BEGINSWITH %@", "Dictate in")), escape, Match(.button, "Control Return")
        ]
        for orientation: UIDeviceOrientation in [.landscapeLeft, .portrait] {
            XCTAssertTrue(app.rotate(to: orientation), "\(orientation.rawValue)\n\(app.debugDescription)")
            XCTAssertTrue(keyboard.appears(within: 30), "\(orientation.rawValue)\n\(app.debugDescription)")
            XCTAssertTrue(shown(controls, app) { screen, frames in
                guard let escape = screen[escape]?.frame, let top = screen[terminal]?.frame.minY else { return false }
                return aboveKeyboard(frames, app) && escape.minY - max(top, screen[header]?.frame.maxY ?? 0) > 24
            }, "\(orientation.rawValue)\n\(app.debugDescription)")
        }
        app.withInteractionOptions(2) { app.buttons["Hide keyboard"].tap() }
        XCTAssertTrue(keyboard.disappears(within: 10), app.debugDescription)
        app.withInteractionOptions(2) { app.buttons["Chat"].firstMatch.tap() }
        let latest = Match(.staticText, NSPredicate(format: "label BEGINSWITH %@", "Round 20:"))
        XCTAssertTrue(shown([latest], app), app.debugDescription)
        XCTAssertTrue(app.rotate(to: .landscapeLeft), app.debugDescription)
        XCTAssertTrue(shown([latest], app), app.debugDescription)
    }

    func testApprovalBandLeavesTheComposerReachableAndTheReviewSheetReportsAFailedDecision() throws {
        let app = try launchApproval()
        defer { app.terminate() }
        // The Chat test above checks that a session without an approval shows it.
        XCTAssertEqual(app.elapsedTimes.count, 0, app.debugDescription)
        // Chat's scroll view takes accessibility hit tests under the header and the band.
        app.buttons["Terminal"].firstMatch.tap()
        XCTAssertTrue(app.rotate(to: .landscapeLeft), app.debugDescription)
        // A band: the tool, the whole command on its row, Review and the decisions.
        let showKeyboard = Match(.button, "Keyboard")
        let reviewButton = Match(.button, "Review")
        XCTAssertTrue(shown([showKeyboard, reviewButton, Match(.staticText, command), Match(.button, "Approve"), Match(.button, "Deny")], app), app.debugDescription)
        app.element(showKeyboard).tap()
        let keyboard = app.keyboards.firstMatch
        XCTAssertTrue(keyboard.appears(within: 10), app.debugDescription)
        let hideKeyboard = Match(.button, "Hide keyboard")
        XCTAssertTrue(shown([reviewButton, hideKeyboard], app) { _, frames in aboveKeyboard([frames[1]], app) }, app.debugDescription)
        app.withInteractionOptions(2) { app.buttons["Hide keyboard"].tap() }
        XCTAssertTrue(keyboard.disappears(within: 10), app.debugDescription)
        try review(app)

        XCTAssertTrue(app.rotate(to: .portrait), app.debugDescription)
        app.buttons["Chat"].firstMatch.tap()
        // In portrait a card: it leaves the output to the sheet, and Review is its one way in.
        XCTAssertTrue(shown([reviewButton], app), app.debugDescription)
        XCTAssertGreaterThanOrEqual(app.element(reviewButton).frame.height, 44, app.debugDescription)
        app.element(reviewButton).tap()
        let close = app.buttons["Close"]
        XCTAssertTrue(eventually(within: 10) { close.exists && close.isHittable }, app.debugDescription)
        // The band stays under the sheet, so each button is looked for among the ones a tap reaches.
        let hittable = { (name: String) in app.buttons.matching(identifier: name).allElementsBoundByIndex.first { $0.isHittable } }
        // A note goes with the decision: Add note sits right above the decisions.
        let sheetNote = try XCTUnwrap(hittable("Add note"), app.debugDescription)
        XCTAssertLessThanOrEqual(sheetNote.frame.maxY, try XCTUnwrap(hittable("Approve")).frame.minY, app.debugDescription)
        XCTAssertGreaterThanOrEqual(sheetNote.frame.height, 44, app.debugDescription)
        // At half height the card above still shows the command: the sheet leaves it there. Landscape shows it in both.
        let commands = app.staticTexts.matching(NSPredicate(format: "label == %@", command))
        XCTAssertEqual(commands.count, 1, app.debugDescription)
        // The demo's authentication fails and nothing is sent: the notice must show in the sheet, not only on the band behind it.
        try XCTUnwrap(hittable("Approve"), app.debugDescription).tap()
        let notice = app.staticTexts.matching(NSPredicate(format: "label == %@", "Not authenticated. Nothing was sent."))
        XCTAssertTrue(eventually { notice.allElementsBoundByIndex.contains { $0.isHittable } }, app.debugDescription)
        // The card's copy hides while the sheet shows its own, which it used to peek above.
        XCTAssertEqual(notice.count, 1, app.debugDescription)
        // In landscape Add note leads the decisions' row, which leaves the output more room.
        XCTAssertTrue(app.rotate(to: .landscapeLeft), app.debugDescription)
        XCTAssertTrue(eventually(within: 10) {
            guard let note = hittable("Add note"), let approve = hittable("Approve") else { return false }
            return note.frame.maxX <= approve.frame.minX && abs(note.frame.midY - approve.frame.midY) < 1
        }, app.debugDescription)
        XCTAssertEqual(commands.count, 2, app.debugDescription)
        XCTAssertTrue(app.rotate(to: .portrait), app.debugDescription)

        // One tap starts the note: the field shows focused, and it and the decisions stay above the keyboard.
        XCTAssertTrue(eventually(within: 10) { hittable("Add note") != nil }, app.debugDescription)
        try XCTUnwrap(hittable("Add note"), app.debugDescription).tap()
        let note = app.textFields["Note for Claude Code"]
        XCTAssertTrue(app.keyboards.firstMatch.appears(within: 10), app.debugDescription)
        XCTAssertTrue(eventually(within: 10) { note.isHittable && hittable("Approve") != nil }, app.debugDescription)
        // The keyboard lifts the sheet over the card, so the sheet shows the command again.
        XCTAssertTrue(eventually(within: 10) { commands.count == 2 }, app.debugDescription)
        _ = app.type("tests only", into: note)
        XCTAssertTrue(app.rotate(to: .landscapeLeft), app.debugDescription)
        XCTAssertTrue(eventually(within: 10) { note.isHittable && hittable("Approve") != nil }, app.debugDescription)
    }

    func testLargeTextKeepsTheApprovalDecisionsAndTheKeysInReach() throws {
        let app = try launchApproval(["-UIPreferredContentSizeCategoryName", "UICTContentSizeCategoryAccessibilityXL"])
        defer { app.terminate() }
        // Chat's scroll view takes accessibility hit tests under the header and the band.
        app.buttons["Terminal"].firstMatch.tap()
        XCTAssertTrue(eventually(within: 30) { app.buttons["Approve"].isHittable && app.buttons["Review"].isHittable }, app.debugDescription)
        XCTAssertTrue(eventually(within: 30) { app.buttons["Keyboard"].isHittable }, app.debugDescription)

        // Delivered to the running app: XCUIApplication.open would launch it again.
        XCUIDevice.shared.system.open(try XCTUnwrap(URL(string: "collie://agent?m=demo&t=term_demo")))
        XCTAssertTrue(app.navigationBars.staticTexts["fix the build"].appears(within: 10), app.debugDescription)
        app.buttons["Terminal"].firstMatch.tap()
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
    }

    func testHalfHeightReviewKeepsACommandTheCardCuts() throws {
        continueAfterFailure = false
        let app = try demoApp("landscape-demo", ["--approval-demo"])
        let long = "cargo test -p collied --test approvals --features herdr-core,notifications -- --nocapture --test-threads 1 "
            + "approvals::decide approvals::expire approvals::note"
        app.launchEnvironment["COLLIE_DEMO_APPROVAL_COMMAND"] = long
        try app.launch(onSession: "term_2")
        defer { app.terminate() }
        let review = app.buttons["Review"]
        XCTAssertTrue(review.appears(within: 10), app.debugDescription)
        review.tap()
        let close = app.buttons["Close"]
        XCTAssertTrue(eventually(within: 10) { close.exists && close.isHittable }, app.debugDescription)
        // The card cuts the command after three lines, so the half-height sheet shows it whole. The card shows the reason
        // whole and the session header the task, so the sheet leaves them out.
        let commands = app.staticTexts.matching(NSPredicate(format: "label == %@", long))
        XCTAssertTrue(eventually(within: 10) { commands.count == 2 }, app.debugDescription)
        let inSheet = { (query: XCUIElementQuery) in query.allElementsBoundByIndex.contains { $0.frame.minY > close.frame.maxY } }
        let reason = app.descendants(matching: .any).matching(NSPredicate(format: "label BEGINSWITH %@", "Reason: "))
        XCTAssertTrue(reason.firstMatch.exists, app.debugDescription)
        XCTAssertFalse(inSheet(reason), app.debugDescription)
        XCTAssertFalse(inSheet(app.descendants(matching: .any).matching(NSPredicate(format: "label == %@", "add approval tests"))), app.debugDescription)
    }

    /// Opens the review sheet from the band: the task, the whole command and every decision in reach, and the reason at
    /// most a scroll away, then closes it. The band stays under the sheet, so each is looked for among the elements a tap reaches.
    private func review(_ app: XCUIApplication) throws {
        app.buttons["Review"].tap()
        let close = app.buttons["Close"]
        XCTAssertTrue(eventually(within: 10) { close.exists && close.isHittable }, app.debugDescription)
        let task = app.descendants(matching: .any).matching(NSPredicate(format: "label == %@", "add approval tests"))
        let reason = app.descendants(matching: .any).matching(NSPredicate(format: "label BEGINSWITH %@", "Reason: "))
        let commands = app.staticTexts.matching(NSPredicate(format: "label == %@", command))
        let decisions = ["Approve", "Always allow", "Deny"].map { ($0, app.buttons.matching(identifier: $0)) }
        let hittable = { (query: XCUIElementQuery) in query.allElementsBoundByIndex.first { $0.isHittable } }
        for (name, query) in [("task", task), ("command", commands)] + decisions {
            XCTAssertTrue(eventually(within: 10) { hittable(query) != nil }, "\(name)\n\(app.debugDescription)")
        }
        // In landscape at accessibility sizes the sheet shows little more than the command over the decisions.
        if hittable(reason) == nil { try XCTUnwrap(hittable(commands), app.debugDescription).swipeUp() }
        XCTAssertTrue(eventually(within: 10) { hittable(reason) != nil }, "reason\n\(app.debugDescription)")
        close.tap()
        XCTAssertTrue(close.disappears(within: 10), app.debugDescription)
    }

    private func launchApproval(_ arguments: [String] = []) throws -> XCUIApplication {
        continueAfterFailure = false
        let app = try demoApp("landscape-demo", ["--approval-demo"] + arguments)
        try app.launch(onSession: "term_2")
        XCTAssertTrue(app.buttons["Approve"].appears(within: 10), app.debugDescription)
        return app
    }

    /// On screen: the same frame inside the main window on two snapshots a poll apart, accepted by `check`, then hittable.
    /// Frames lag the rotation, and asking for hittability mid-rotation fails the test.
    private func shown(
        _ matches: [Match], _ app: XCUIApplication, within timeout: TimeInterval = 30,
        where check: (Screen, [CGRect]) -> Bool = { _, _ in true }
    ) -> Bool {
        var last = app.screen().map { screen in matches.map { screen[$0]?.frame ?? .null } } ?? []
        usleep(250_000)
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
