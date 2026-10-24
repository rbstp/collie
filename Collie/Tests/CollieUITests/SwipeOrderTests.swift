import XCTest

@MainActor
final class SwipeOrderTests: XCTestCase {
    private let titles = ["fix the build", "add approval tests", "update the docs", "dark mode", "rate limiter", "triage issues"]

    func testSwipingFollowsTheGridsCustomOrder() throws {
        continueAfterFailure = false
        let app = try launch()
        defer { app.terminate() }
        let layout = app.buttons["Layout"]
        app.choose(app.buttons["Grid"]) { layout.tap() }
        let before = gridOrder(app)
        XCTAssertEqual(before.count, titles.count, app.debugDescription)
        let moved = try XCTUnwrap(before.last)
        let source = card(app, moved)
        let target = card(app, before[0])
        source.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).press(
            forDuration: 1.5, thenDragTo: target.coordinate(withNormalizedOffset: CGVector(dx: 0.05, dy: 0.5)),
            withVelocity: .slow, thenHoldForDuration: 1
        )
        let deadline = Date.now.addingTimeInterval(10)
        while Date.now < deadline, gridOrder(app).first != moved { usleep(250_000) }
        let order = gridOrder(app)
        XCTAssertEqual(order.first, moved, app.debugDescription)
        XCTAssertEqual(Set(order), Set(before), app.debugDescription)

        card(app, order[0]).tap()
        try expectSwipes(app, through: order)
    }

    func testSwipingFollowsTheFilteredInboxOrder() throws {
        continueAfterFailure = false
        let app = try launch()
        defer { app.terminate() }
        let layout = app.buttons["Layout"]
        app.choose(app.buttons["Inbox"]) { layout.tap() }
        let inactive = app.segmentedControls["Filter"].buttons["Inactive"]
        XCTAssertTrue(inactive.appears(within: 5), app.debugDescription)
        inactive.tap()
        // Most recent activity first, the reverse of the list's longest-idle-first order.
        let rows = ["The docs now describe the inbox and the context ring.", "triage issues", "The rate limiter is merged."]
        let frames = rows.map { app.staticTexts[$0].firstMatch }.map { row in
            XCTAssertTrue(row.appears(within: 5), app.debugDescription)
            return row.frame.minY
        }
        XCTAssertEqual(frames, frames.sorted(), app.debugDescription)
        XCTAssertFalse(app.staticTexts["Dark mode follows the system setting on every page."].exists, app.debugDescription)

        app.staticTexts[rows[0]].firstMatch.tap()
        try expectSwipes(app, through: ["update the docs", "triage issues", "rate limiter"])
    }

    private func launch() throws -> XCUIApplication {
        let app = XCUIApplication()
        let fixture = try XCTUnwrap(Bundle(for: Self.self).url(forResource: "attachment-demo", withExtension: "txt"))
        app.launchArguments = ["-AppleLanguages", "(en)", "-AppleLocale", "en_US", "--terminal-demo", fixture.path]
        app.launch()
        XCTAssertTrue(app.buttons["Layout"].appears(within: 10), app.debugDescription)
        return app
    }

    private func card(_ app: XCUIApplication, _ title: String) -> XCUIElement {
        app.buttons.matching(NSPredicate(format: "label CONTAINS %@", title)).firstMatch
    }

    /// The grid's titles as it shows them: by row, then left to right.
    private func gridOrder(_ app: XCUIApplication) -> [String] {
        titles.compactMap { title in
            let card = card(app, title)
            return card.appears(within: 5) ? (title, card.frame) : nil
        }
        .sorted { ($0.1.minY.rounded(), $0.1.minX) < ($1.1.minY.rounded(), $1.1.minX) }
        .map(\.0)
    }

    /// Swipes left from the first agent to the last, checks the last has no next agent, then swipes back once.
    private func expectSwipes(_ app: XCUIApplication, through order: [String]) throws {
        for (index, title) in order.enumerated() {
            try showTerminal(app, title)
            if index < order.count - 1 { swipe(app, left: true) }
        }
        swipe(app, left: true)
        sleep(2)
        try showTerminal(app, order[order.count - 1])
        swipe(app, left: false)
        try showTerminal(app, order[order.count - 2])
    }

    private func showTerminal(_ app: XCUIApplication, _ title: String) throws {
        let header = app.navigationBars.staticTexts[title]
        XCTAssertTrue(header.appears(within: 10), app.debugDescription)
        let terminal = app.buttons["Terminal"].firstMatch
        XCTAssertTrue(terminal.appears(within: 5), app.debugDescription)
        terminal.tap()
    }

    private func swipe(_ app: XCUIApplication, left: Bool) {
        let start = app.coordinate(withNormalizedOffset: CGVector(dx: left ? 0.8 : 0.2, dy: 0.5))
        let end = app.coordinate(withNormalizedOffset: CGVector(dx: left ? 0.2 : 0.8, dy: 0.5))
        start.press(forDuration: 0.05, thenDragTo: end, withVelocity: .fast, thenHoldForDuration: 0)
    }
}
