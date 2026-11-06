import XCTest

@MainActor
final class SessionHeaderTests: XCTestCase {
    func testElapsedTimeHidesWhileAnApprovalShows() throws {
        continueAfterFailure = false
        let app = try demoApp("landscape-demo", ["--approval-demo"])
        app.launch()
        defer { app.terminate() }
        XCTAssertTrue(app.buttons["New task"].appears(within: 10), app.debugDescription)
        let elapsed = app.staticTexts.matching(NSPredicate(format: "label MATCHES %@", "[0-9]+[smd]|[0-9]+h [0-9]+m"))
        app.open(try XCTUnwrap(URL(string: "collie://agent?m=demo&t=term_demo")))
        XCTAssertTrue(elapsed.firstMatch.appears(within: 10), app.debugDescription)
        app.open(try XCTUnwrap(URL(string: "collie://agent?m=demo&t=term_2")))
        XCTAssertTrue(app.buttons["Approve"].appears(within: 10), app.debugDescription)
        XCTAssertEqual(elapsed.count, 0, app.debugDescription)
    }
}
