import XCTest

@MainActor
final class LandscapeTests: XCTestCase {
    // An async tearDown makes XCTest end the runner on a failure when continueAfterFailure is false, and xcodebuild then
    // skips -retry-tests-on-failure for the test.
    override func tearDown() {
        MainActor.assumeIsolated { XCUIDevice.shared.orientation = .portrait }
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
        let usage = Match(.staticText, "5-hour limit")
        XCTAssertTrue(app.element(usage).appears(within: 10), app.debugDescription)
        for orientation: UIDeviceOrientation in [.landscapeRight, .portrait, .landscapeLeft] {
            XCTAssertTrue(app.rotate(to: orientation), app.debugDescription)
            XCTAssertTrue(shown([usage], app), app.debugDescription)
            XCTAssertTrue(view.buttons["Usage"].isSelected, app.debugDescription)
        }
        app.buttons["Machines"].tap()
        XCTAssertTrue(shown([Match(.staticText, "MacBook Pro")], app), app.debugDescription)
    }

    func testOneMachineLeavesItsNameOffAgentsButKeepsTheApprovalsPill() throws {
        let app = try launchDemo(["--approval-demo"])
        defer { app.terminate() }
        for layout in ["List", "Grid"] {
            app.choose(app.buttons[layout]) { app.buttons["Layout"].tap() }
            XCTAssertTrue(app.descendants(matching: .any)["1 approval"].appears(within: 10), "\(layout)\n\(app.debugDescription)")
            XCTAssertFalse(app.staticTexts["MacBook Pro"].exists, "\(layout)\n\(app.debugDescription)")
        }
    }

    func testSessionKeepsKeyboardAndLatestOutputAcrossRotation() throws {
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
        let showKeyboard = Match(.button, "Keyboard")
        XCTAssertTrue(shown([showKeyboard], app), app.debugDescription)
        app.element(showKeyboard).tap()
        let keyboard = app.keyboards.firstMatch
        XCTAssertTrue(keyboard.appears(within: 10), app.debugDescription)
        let escape = Match(.button, "Escape")
        let header = Match(.button, "Terminal")
        let controls = [
            Match(.button, "Hide keyboard"), Match(.button, "Attachments and saved prompts"),
            Match(.button, NSPredicate(format: "label BEGINSWITH %@", "Dictate in")), escape, Match(.button, "Control Return")
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
        app.withInteractionOptions(2) { app.buttons["Chat"].firstMatch.tap() }
        let latest = Match(.staticText, NSPredicate(format: "label BEGINSWITH %@", "Round 20:"))
        XCTAssertTrue(shown([latest], app), app.debugDescription)
        for orientation: UIDeviceOrientation in [.portrait, .landscapeLeft] {
            XCTAssertTrue(app.rotate(to: orientation), "\(orientation.rawValue)\n\(app.debugDescription)")
            XCTAssertTrue(shown([latest], app), "\(orientation.rawValue)\n\(app.debugDescription)")
        }
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

    func testApprovalBandLeavesTheComposerReachable() throws {
        let app = try launchDemo(["--approval-demo"])
        defer { app.terminate() }
        app.open(try XCTUnwrap(URL(string: "collie://agent?m=demo&t=term_2")))
        XCTAssertTrue(app.buttons["Approve"].appears(within: 10), app.debugDescription)
        // Chat's scroll view takes accessibility hit tests under the header and the band.
        app.buttons["Terminal"].firstMatch.tap()
        XCTAssertTrue(app.rotate(to: .landscapeLeft), app.debugDescription)
        // A band: the tool, the whole command on its row, Review and the decisions.
        let showKeyboard = Match(.button, "Keyboard")
        let reviewButton = Match(.button, "Review")
        let command = Match(.staticText, "cargo test -p collied --test approvals -- --nocapture")
        XCTAssertTrue(shown([showKeyboard, reviewButton, command, Match(.button, "Approve"), Match(.button, "Deny")], app), app.debugDescription)
        app.element(showKeyboard).tap()
        let keyboard = app.keyboards.firstMatch
        XCTAssertTrue(keyboard.appears(within: 10), app.debugDescription)
        let hideKeyboard = Match(.button, "Hide keyboard")
        XCTAssertTrue(shown([hideKeyboard], app) { _, frames in aboveKeyboard(frames, app) }, app.debugDescription)
        XCTAssertTrue(shown([reviewButton, hideKeyboard], app), app.debugDescription)
        app.withInteractionOptions(2) { app.buttons["Hide keyboard"].tap() }
        XCTAssertTrue(keyboard.disappears(within: 10), app.debugDescription)
        try review(app)
    }

    func testApprovalBandKeepsApproveInReachAtLargeText() throws {
        let app = try launchDemo(["--approval-demo", "-UIPreferredContentSizeCategoryName", "UICTContentSizeCategoryAccessibilityXL"])
        defer { app.terminate() }
        app.open(try XCTUnwrap(URL(string: "collie://agent?m=demo&t=term_2")))
        let approve = app.buttons["Approve"]
        XCTAssertTrue(approve.appears(within: 10), app.debugDescription)
        // Chat's scroll view takes accessibility hit tests under the header and the band.
        app.buttons["Terminal"].firstMatch.tap()
        XCTAssertTrue(eventually(within: 30) { approve.isHittable && app.buttons["Review"].isHittable }, app.debugDescription)
        XCTAssertTrue(eventually(within: 30) { app.buttons["Keyboard"].isHittable }, app.debugDescription)
        try review(app)
    }

    func testApprovalBandOpensTheReviewInLandscapeAtLargeText() throws {
        let app = try launchDemo(["--approval-demo", "-UIPreferredContentSizeCategoryName", "UICTContentSizeCategoryAccessibilityXL"])
        defer { app.terminate() }
        app.open(try XCTUnwrap(URL(string: "collie://agent?m=demo&t=term_2")))
        XCTAssertTrue(app.buttons["Review"].appears(within: 10), app.debugDescription)
        app.buttons["Terminal"].firstMatch.tap()
        XCTAssertTrue(app.rotate(to: .landscapeLeft), app.debugDescription)
        XCTAssertTrue(shown([Match(.button, "Review"), Match(.button, "Keyboard")], app), app.debugDescription)
        try review(app)
    }

    func testReviewSheetReportsAFailedDecision() throws {
        let app = try launchDemo(["--approval-demo"])
        defer { app.terminate() }
        app.open(try XCTUnwrap(URL(string: "collie://agent?m=demo&t=term_2")))
        XCTAssertTrue(app.buttons["Approve"].appears(within: 10), app.debugDescription)
        // The card leaves the output to the sheet, and Review is its one way in.
        let review = app.buttons["Review"]
        XCTAssertTrue(review.appears(within: 10), app.debugDescription)
        XCTAssertFalse(app.staticTexts["test approvals::case_1 ... ok"].exists, app.debugDescription)
        XCTAssertFalse(app.buttons.matching(NSPredicate(format: "label BEGINSWITH %@", "Output")).firstMatch.exists, app.debugDescription)
        // Add note lives in the sheet only.
        XCTAssertFalse(app.buttons["Add note"].exists, app.debugDescription)
        XCTAssertGreaterThanOrEqual(review.frame.height, 44, app.debugDescription)
        review.tap()
        let close = app.buttons["Close"]
        XCTAssertTrue(eventually(within: 10) { close.exists && close.isHittable }, app.debugDescription)
        // A note goes with the decision: Add note sits right above the decisions.
        let sheetNote = app.buttons.matching(identifier: "Add note").allElementsBoundByIndex.first { $0.isHittable }
        let sheetApprove = app.buttons.matching(identifier: "Approve").allElementsBoundByIndex.first { $0.isHittable }
        XCTAssertLessThanOrEqual(try XCTUnwrap(sheetNote, app.debugDescription).frame.maxY, try XCTUnwrap(sheetApprove).frame.minY, app.debugDescription)
        XCTAssertGreaterThanOrEqual(try XCTUnwrap(sheetNote).frame.height, 44, app.debugDescription)
        // At half height the card above still shows the command: the sheet leaves it there. Landscape shows it in both.
        let command = app.staticTexts.matching(NSPredicate(format: "label == %@", "cargo test -p collied --test approvals -- --nocapture"))
        XCTAssertEqual(command.count, 1, app.debugDescription)
        // Authentication fails (cancelled, or no passcode set) and nothing is sent: the notice must show in the sheet, not
        // only on the band behind it.
        let approve = app.buttons.matching(identifier: "Approve").allElementsBoundByIndex.first { $0.isHittable }
        try XCTUnwrap(approve, app.debugDescription).tap()
        let cancel = XCUIApplication(bundleIdentifier: "com.apple.springboard").buttons["Cancel"]
        if cancel.waitForExistence(timeout: 5) { cancel.tap() }
        let notice = app.staticTexts.matching(NSPredicate(format: "label == %@", "Not authenticated. Nothing was sent."))
        XCTAssertTrue(eventually(within: 10) { notice.allElementsBoundByIndex.contains { $0.isHittable } }, app.debugDescription)
        // It stays until the next decision, past the six seconds it used to last.
        sleep(7)
        XCTAssertTrue(notice.allElementsBoundByIndex.contains { $0.isHittable }, app.debugDescription)
        // The card's copy hides while the sheet shows its own, which it used to peek above.
        XCTAssertEqual(notice.count, 1, app.debugDescription)
        // In landscape Add note leads the decisions' row, which leaves the output more room.
        XCTAssertTrue(app.rotate(to: .landscapeLeft), app.debugDescription)
        XCTAssertTrue(eventually(within: 10) {
            guard let note = app.buttons.matching(identifier: "Add note").allElementsBoundByIndex.first(where: { $0.isHittable }),
                  let approve = app.buttons.matching(identifier: "Approve").allElementsBoundByIndex.first(where: { $0.isHittable }) else { return false }
            return note.frame.maxX <= approve.frame.minX && abs(note.frame.midY - approve.frame.midY) < 1
        }, app.debugDescription)
        XCTAssertEqual(command.count, 2, app.debugDescription)
        XCTAssertTrue(app.rotate(to: .portrait), app.debugDescription)
        close.tap()
        XCTAssertTrue(close.disappears(within: 10), app.debugDescription)
        XCTAssertTrue(notice.firstMatch.exists, app.debugDescription)
        // Another task never shows this approval's notice.
        app.open(try XCTUnwrap(URL(string: "collie://agent?m=demo&t=term_3")))
        XCTAssertTrue(app.navigationBars.staticTexts["update the docs"].appears(within: 10), app.debugDescription)
        XCTAssertFalse(app.buttons["Approve"].exists, app.debugDescription)
        XCTAssertFalse(notice.firstMatch.exists, app.debugDescription)
    }

    func testHalfHeightReviewKeepsACommandTheCardCuts() throws {
        continueAfterFailure = false
        let app = try demoApp("landscape-demo", ["--approval-demo"])
        let long = "cargo test -p collied --test approvals --features herdr-core,notifications -- --nocapture --test-threads 1 "
            + "approvals::decide approvals::expire approvals::note"
        app.launchEnvironment["COLLIE_DEMO_APPROVAL_COMMAND"] = long
        app.launch()
        defer { app.terminate() }
        XCTAssertTrue(app.buttons["New task"].appears(within: 10), app.debugDescription)
        app.open(try XCTUnwrap(URL(string: "collie://agent?m=demo&t=term_2")))
        let review = app.buttons["Review"]
        XCTAssertTrue(review.appears(within: 10), app.debugDescription)
        review.tap()
        let close = app.buttons["Close"]
        XCTAssertTrue(eventually(within: 10) { close.exists && close.isHittable }, app.debugDescription)
        // The card cuts the command after three lines, so the half-height sheet shows it whole. The card shows the reason
        // whole and the session header the task, so the sheet leaves them out.
        let command = app.staticTexts.matching(NSPredicate(format: "label == %@", long))
        XCTAssertTrue(eventually(within: 10) { command.count == 2 }, app.debugDescription)
        let inSheet = { (query: XCUIElementQuery) in query.allElementsBoundByIndex.contains { $0.frame.minY > close.frame.maxY } }
        let reason = app.descendants(matching: .any).matching(NSPredicate(format: "label BEGINSWITH %@", "Reason: "))
        XCTAssertTrue(reason.firstMatch.exists, app.debugDescription)
        XCTAssertFalse(inSheet(reason), app.debugDescription)
        XCTAssertFalse(inSheet(app.descendants(matching: .any).matching(NSPredicate(format: "label == %@", "add approval tests"))), app.debugDescription)
    }

    func testAddNoteFocusesTheNoteFieldAboveTheKeyboard() throws {
        let app = try launchDemo(["--approval-demo"])
        defer { app.terminate() }
        app.open(try XCTUnwrap(URL(string: "collie://agent?m=demo&t=term_2")))
        let review = app.buttons["Review"]
        XCTAssertTrue(review.appears(within: 10), app.debugDescription)
        review.tap()
        let hittable = { (name: String) in app.buttons.matching(identifier: name).allElementsBoundByIndex.first { $0.isHittable } }
        XCTAssertTrue(eventually(within: 10) { hittable("Add note") != nil }, app.debugDescription)
        try XCTUnwrap(hittable("Add note"), app.debugDescription).tap()
        // One tap starts the note: the field shows focused, and it and the decisions stay above the keyboard.
        let note = app.textFields["Note for Claude Code"]
        XCTAssertTrue(app.keyboards.firstMatch.appears(within: 10), app.debugDescription)
        XCTAssertTrue(eventually(within: 10) { note.isHittable && hittable("Approve") != nil }, app.debugDescription)
        // The keyboard lifts the sheet over the card, so the sheet shows the command again.
        let command = app.staticTexts.matching(NSPredicate(format: "label == %@", "cargo test -p collied --test approvals -- --nocapture"))
        XCTAssertTrue(eventually(within: 10) { command.count == 2 }, app.debugDescription)
        _ = app.type("tests only", into: note)
        XCTAssertTrue(app.rotate(to: .landscapeLeft), app.debugDescription)
        XCTAssertTrue(eventually(within: 10) { note.isHittable && hittable("Approve") != nil }, app.debugDescription)
        XCTAssertTrue(app.rotate(to: .portrait), app.debugDescription)
    }

    /// Opens the review sheet from the band: the task, the whole command and every decision in reach, and the reason at
    /// most a scroll away, then closes it. The band stays under the sheet, so each is looked for among the elements a tap reaches.
    private func review(_ app: XCUIApplication) throws {
        app.buttons["Review"].tap()
        let close = app.buttons["Close"]
        XCTAssertTrue(eventually(within: 10) { close.exists && close.isHittable }, app.debugDescription)
        let task = app.descendants(matching: .any).matching(NSPredicate(format: "label == %@", "add approval tests"))
        let reason = app.descendants(matching: .any).matching(NSPredicate(format: "label BEGINSWITH %@", "Reason: "))
        let command = app.staticTexts.matching(NSPredicate(format: "label == %@", "cargo test -p collied --test approvals -- --nocapture"))
        let decisions = ["Approve", "Always allow", "Deny"].map { ($0, app.buttons.matching(identifier: $0)) }
        let hittable = { (query: XCUIElementQuery) in query.allElementsBoundByIndex.first { $0.isHittable } }
        for (name, query) in [("task", task), ("command", command)] + decisions {
            XCTAssertTrue(eventually(within: 10) { hittable(query) != nil }, "\(name)\n\(app.debugDescription)")
        }
        // In landscape at accessibility sizes the sheet shows little more than the command over the decisions.
        if !eventually(within: 2, { hittable(reason) != nil }) { try XCTUnwrap(hittable(command), app.debugDescription).swipeUp() }
        XCTAssertTrue(eventually(within: 10) { hittable(reason) != nil }, "reason\n\(app.debugDescription)")
        close.tap()
        XCTAssertTrue(close.disappears(within: 10), app.debugDescription)
    }

    private func launchDemo(_ arguments: [String] = []) throws -> XCUIApplication {
        continueAfterFailure = false
        let app = try demoApp("landscape-demo", arguments)
        app.launch()
        XCTAssertTrue(app.buttons["New task"].appears(within: 10), app.debugDescription)
        return app
    }

    /// On screen: the same frame inside the main window on two snapshots a poll apart, accepted by `check`, then hittable.
    /// Frames lag the rotation, and asking for hittability mid-rotation fails the test. A zero timeout checks once.
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
