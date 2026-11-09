import XCTest

@MainActor
final class LimitTests: XCTestCase {
    func testALimitedAgentShowsItsResumeAndCancelsIt() throws {
        continueAfterFailure = false
        let app = try demoApp("landscape-demo", ["--limit-demo"])
        app.launch()
        defer { app.terminate() }
        XCTAssertTrue(app.buttons["New task"].appears(within: 10), app.debugDescription)
        app.open(try XCTUnwrap(URL(string: "collie://agent?m=demo&t=term_3")))
        let armed = app.buttons.matching(NSPredicate(format: "label BEGINSWITH %@", "Limited at the 5-hour limit, resumes at")).firstMatch
        XCTAssertTrue(armed.appears(within: 10), app.debugDescription)
        armed.tap()
        let cancel = app.buttons["Cancel resume"]
        XCTAssertTrue(cancel.appears(within: 5), app.debugDescription)
        cancel.tap()
        let cancelled = app.buttons.matching(NSPredicate(format: "label ENDSWITH %@", "will not resume")).firstMatch
        XCTAssertTrue(cancelled.appears(within: 10), app.debugDescription)
        cancelled.tap()
        XCTAssertTrue(app.buttons["Continue now"].appears(within: 5), app.debugDescription)
        XCTAssertFalse(app.buttons["Cancel resume"].exists, app.debugDescription)
    }
}
