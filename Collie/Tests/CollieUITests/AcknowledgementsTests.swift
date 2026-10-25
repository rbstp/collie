import XCTest

@MainActor
final class AcknowledgementsTests: XCTestCase {
    func testAcknowledgementsShowLibtailscaleLicense() throws {
        let app = XCUIApplication()
        app.launchArguments = ["-AppleLanguages", "(en)", "-AppleLocale", "en_US"]
        app.launch()
        defer { app.terminate() }
        if app.buttons["Settings"].exists { app.buttons["Settings"].tap() }
        XCTAssertTrue(app.buttons["Explore sample workspace"].appears(within: 10))
        try XCTSkipIf(app.staticTexts["collie could not start"].exists, "an unsigned build has no keychain, so no Welcome screen")
        let link = app.buttons["Acknowledgements"]
        for _ in 0..<8 where !(link.exists && link.isHittable) { app.swipeUp() }
        link.tap()
        let libtailscale = app.buttons.matching(NSPredicate(format: "label BEGINSWITH %@", "libtailscale")).firstMatch
        XCTAssertTrue(libtailscale.appears(within: 5))
        libtailscale.tap()
        let notice = app.staticTexts.containing(NSPredicate(format: "label CONTAINS %@", "Tailscale & AUTHORS")).firstMatch
        XCTAssertTrue(notice.appears(within: 5))
    }
}
