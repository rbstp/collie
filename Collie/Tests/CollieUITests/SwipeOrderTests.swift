import XCTest

/// The neighbor rules (ends, closed agents, machines) are unit tested on FlockOrder: here, the layout's order reaches the
/// session and the swipe follows it.
@MainActor
final class SwipeOrderTests: XCTestCase {
    private let titles = ["fix the build", "add approval tests", "update the docs", "dark mode", "rate limiter", "triage issues"]

    func testSwipingFollowsTheGridsCustomOrderAndTheFilteredInbox() throws {
        continueAfterFailure = false
        let app = try demoApp("attachment-demo")
        app.launch()
        defer { app.terminate() }
        let layout = app.buttons["Layout"]
        XCTAssertTrue(layout.appears(within: 10), app.debugDescription)
        app.choose(app.buttons["Grid"]) { layout.tap() }
        _ = eventually { gridOrder(app).count == titles.count }
        let before = gridOrder(app)
        XCTAssertEqual(before.count, titles.count, app.debugDescription)
        let moved = try XCTUnwrap(before.last)
        drag(app, moved, onto: before[0], after: false)
        _ = eventually { gridOrder(app).first == moved }
        let order = gridOrder(app)
        XCTAssertEqual(order.first, moved, app.debugDescription)
        XCTAssertEqual(Set(order), Set(before), app.debugDescription)

        card(app, order[0]).tap()
        expectSwipe(app, from: order[0], to: order[1])

        // The order is kept on the simulator, and other tests expect the first cards on screen.
        app.navigationBars.buttons.element(boundBy: 0).tap()
        XCTAssertTrue(card(app, moved).appears(within: 10), app.debugDescription)
        drag(app, moved, onto: before[before.count - 2], after: true)
        _ = eventually { gridOrder(app) == before }
        XCTAssertEqual(gridOrder(app), before, app.debugDescription)

        app.choose(app.buttons["Inbox"]) { layout.tap() }
        let inactive = app.segmentedControls["Filter"].buttons["Inactive"]
        XCTAssertTrue(inactive.appears(within: 5), app.debugDescription)
        let header = app.collectionViews.firstMatch.collectionViews.staticTexts["Needs you"]
        XCTAssertTrue(header.appears(within: 5), app.debugDescription)
        app.segmentedControls["Filter"].buttons["Needs you"].tap()
        // The selected filter names the one group, so its header hides.
        XCTAssertTrue(eventually { !header.exists }, app.debugDescription)
        inactive.tap()
        // Most recent activity first, the reverse of the list's longest-idle-first order.
        let first = app.staticTexts["The docs now describe the inbox and the context ring."].firstMatch
        XCTAssertTrue(first.appears(within: 5), app.debugDescription)
        XCTAssertFalse(app.staticTexts["Dark mode follows the system setting on every page."].exists, app.debugDescription)
        first.tap()
        // Without the filter, dark mode comes between these two.
        expectSwipe(app, from: "update the docs", to: "triage issues")
    }

    private func card(_ app: XCUIApplication, _ title: String) -> XCUIElement {
        app.buttons.matching(NSPredicate(format: "label CONTAINS %@", title)).firstMatch
    }

    private func drag(_ app: XCUIApplication, _ title: String, onto target: String, after: Bool) {
        // On a slow CI simulator the press can settle into the card's context menu instead of lifting the card. The open
        // menu keeps the app from idling, so the press and the dismissing tap skip XCTest's 60 s waits for it (see choose).
        let menu = app.buttons["Close pane"]
        let before = gridOrder(app)
        for _ in 0..<3 {
            app.withInteractionOptions(2) {
                card(app, title).coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).press(
                    forDuration: 1.5, thenDragTo: card(app, target).coordinate(withNormalizedOffset: CGVector(dx: after ? 0.95 : 0.05, dy: 0.5)),
                    withVelocity: .slow, thenHoldForDuration: 1
                )
            }
            _ = eventually(within: 5) { menu.exists || gridOrder(app) != before }
            guard menu.exists else { return }
            app.withInteractionOptions(2) { app.coordinate(withNormalizedOffset: CGVector(dx: 0.15, dy: 0.5)).tap() }
            XCTAssertTrue(menu.disappears(within: 10), app.debugDescription)
        }
    }

    /// The grid's titles as it shows them: by row, then left to right.
    private func gridOrder(_ app: XCUIApplication) -> [String] {
        guard let cards = app.screen()?.elements.filter({ $0.elementType == .button }) else { return [] }
        return titles.compactMap { title in cards.first { $0.label.contains(title) }.map { (title, $0.frame) } }
            .sorted { ($0.1.minY.rounded(), $0.1.minX) < ($1.1.minY.rounded(), $1.1.minX) }
            .map(\.0)
    }

    /// Swipes left from the open session to the next one.
    private func expectSwipe(_ app: XCUIApplication, from first: String, to next: String) {
        XCTAssertTrue(app.navigationBars.staticTexts[first].appears(within: 10), app.debugDescription)
        let terminal = app.buttons["Terminal"].firstMatch
        XCTAssertTrue(terminal.appears(within: 5), app.debugDescription)
        terminal.tap()
        let start = app.coordinate(withNormalizedOffset: CGVector(dx: 0.8, dy: 0.5))
        start.press(forDuration: 0.05, thenDragTo: app.coordinate(withNormalizedOffset: CGVector(dx: 0.2, dy: 0.5)), withVelocity: .fast, thenHoldForDuration: 0)
        XCTAssertTrue(app.navigationBars.staticTexts[next].appears(within: 10), app.debugDescription)
    }
}
