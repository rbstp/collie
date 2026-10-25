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
        drag(app, moved, onto: before[0], after: false)
        var deadline = Date.now.addingTimeInterval(10)
        while Date.now < deadline, gridOrder(app).first != moved { usleep(250_000) }
        let order = gridOrder(app)
        XCTAssertEqual(order.first, moved, app.debugDescription)
        XCTAssertEqual(Set(order), Set(before), app.debugDescription)

        card(app, order[0]).tap()
        try expectSwipes(app, through: order)

        // The order is kept on the simulator, and other tests expect the first cards on screen.
        app.navigationBars.buttons.element(boundBy: 0).tap()
        XCTAssertTrue(card(app, moved).appears(within: 10), app.debugDescription)
        drag(app, moved, onto: before[before.count - 2], after: true)
        deadline = Date.now.addingTimeInterval(10)
        while Date.now < deadline, gridOrder(app) != before { usleep(250_000) }
        XCTAssertEqual(gridOrder(app), before, app.debugDescription)
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

    private func drag(_ app: XCUIApplication, _ title: String, onto target: String, after: Bool) {
        // On a slow CI simulator the press can settle into the card's context menu instead of lifting the card. The open
        // menu keeps the app from idling, so the press and the dismissing tap skip XCTest's 60 s waits for it (see choose).
        let menu = app.buttons["Close pane"]
        for _ in 0..<3 {
            app.withInteractionOptions(2) {
                card(app, title).coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).press(
                    forDuration: 1.5, thenDragTo: card(app, target).coordinate(withNormalizedOffset: CGVector(dx: after ? 0.95 : 0.05, dy: 0.5)),
                    withVelocity: .slow, thenHoldForDuration: 1
                )
            }
            guard menu.appears(within: 2) else { return }
            app.withInteractionOptions(2) { app.coordinate(withNormalizedOffset: CGVector(dx: 0.15, dy: 0.5)).tap() }
            XCTAssertTrue(menu.disappears(within: 10), app.debugDescription)
        }
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
