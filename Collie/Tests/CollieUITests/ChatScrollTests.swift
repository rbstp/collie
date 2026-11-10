import XCTest

@MainActor
final class ChatScrollTests: XCTestCase {
    func testChatOpensAFileOffersLatestMessagesAndExpandsAPullRequestWake() throws {
        continueAfterFailure = false
        let app = try demoApp("landscape-demo")
        try app.launch(onSession: "term_demo")
        defer { app.terminate() }
        let latest = app.staticTexts.matching(NSPredicate(format: "label BEGINSWITH %@", "Round 20:")).firstMatch
        XCTAssertTrue(latest.appears(within: 10), app.debugDescription)
        // LandscapeTests checks that the header leaves the elapsed time out while an approval shows.
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

        let row = app.buttons.matching(NSPredicate(format: "label CONTAINS %@", "PR watch: CI failed on #264")).firstMatch
        XCTAssertTrue(row.appears(within: 5), app.debugDescription)
        let prompt = app.staticTexts.matching(NSPredicate(format: "label BEGINSWITH %@", "Pull request #264")).firstMatch
        XCTAssertFalse(prompt.exists, app.debugDescription)
        row.tap()
        XCTAssertTrue(prompt.appears(within: 5), app.debugDescription)
    }
}
