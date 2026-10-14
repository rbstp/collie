import XCTest

@MainActor
final class PagerSwipeTests: XCTestCase {
    func testSwipeBetweenAgentsAndUsageInEveryLayout() throws {
        continueAfterFailure = false
        let app = XCUIApplication()
        let fixture = try XCTUnwrap(Bundle(for: Self.self).url(forResource: "attachment-demo", withExtension: "txt"))
        app.launchArguments = ["-AppleLanguages", "(en)", "-AppleLocale", "en_US", "--terminal-demo", fixture.path]
        app.launch()
        defer { app.terminate() }
        let layout = app.buttons["Layout"]
        XCTAssertTrue(layout.appears(within: 10), app.debugDescription)
        let view = app.segmentedControls["View"]
        for name in ["Inbox", "Grid", "List", "Inbox", "Grid"] {
            layout.tap()
            app.buttons[name].tap()
            XCTAssertTrue(view.buttons["Agents"].isSelected, app.debugDescription)
            if name == "Inbox" {
                view.buttons["Usage"].tap()
            } else {
                app.swipeLeft()
            }
            XCTAssertTrue(view.buttons["Usage"].wait(for: \.isSelected, toEqual: true, timeout: 5), "\(name)\n\(app.debugDescription)")
            app.swipeRight()
            XCTAssertTrue(view.buttons["Agents"].wait(for: \.isSelected, toEqual: true, timeout: 5), "\(name)\n\(app.debugDescription)")
        }
    }
}
