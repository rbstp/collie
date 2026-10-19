import XCTest

@MainActor
final class ChatScrollTests: XCTestCase {
    func testLatestMessagesShowsOnceScrolledUp() throws {
        continueAfterFailure = false
        let app = XCUIApplication()
        let fixture = try XCTUnwrap(Bundle(for: Self.self).url(forResource: "landscape-demo", withExtension: "txt"))
        app.launchArguments = ["-AppleLanguages", "(en)", "-AppleLocale", "en_US", "--terminal-demo", fixture.path]
        app.launch()
        defer { app.terminate() }
        XCTAssertTrue(app.buttons["New task"].appears(within: 10), app.debugDescription)
        app.open(try XCTUnwrap(URL(string: "collie://agent?m=demo&t=term_demo")))
        let latest = app.staticTexts.matching(NSPredicate(format: "label BEGINSWITH %@", "Round 20:")).firstMatch
        XCTAssertTrue(latest.appears(within: 10), app.debugDescription)
        let button = app.buttons["Latest messages"]
        XCTAssertFalse(button.exists, app.debugDescription)
        let start = app.windows.firstMatch.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.4))
        start.press(forDuration: 0.1, thenDragTo: start.withOffset(CGVector(dx: 0, dy: 250)))
        XCTAssertTrue(button.appears(within: 5), app.debugDescription)
        button.tap()
        XCTAssertTrue(button.disappears(within: 5), app.debugDescription)
    }
}
