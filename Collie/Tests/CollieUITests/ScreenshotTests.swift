import XCTest

/// The documentation screenshots, written only when `just ios-screenshots` sets COLLIE_SCREENSHOTS.
@MainActor
final class ScreenshotTests: XCTestCase {
    func testScreenshots() throws {
        let path = ProcessInfo.processInfo.environment["COLLIE_SCREENSHOTS"] ?? ""
        try XCTSkipIf(path.isEmpty, "set by just ios-screenshots")
        continueAfterFailure = false
        let dir = URL(filePath: path, directoryHint: .isDirectory)
        // just ios-screenshots boots the simulator in this appearance: on iOS 27.2, XCUIDevice.appearance never reaches the app.
        let suffix = try XCTUnwrap(ProcessInfo.processInfo.environment["COLLIE_SCREENSHOTS_APPEARANCE"])
        func save(_ name: String) throws {
            sleep(1)
            try XCUIScreen.main.screenshot().pngRepresentation.write(to: dir.appending(path: "\(name)-\(suffix).png"))
        }

        var app = try launchDemo()
        let card = app.descendants(matching: .any).matching(NSPredicate(format: "label CONTAINS %@", "fix the build")).firstMatch
        XCTAssertTrue(card.appears(within: 10), app.debugDescription)
        try save("agents")
        let shot = try XCTUnwrap(XCUIScreen.main.screenshot().image.cgImage)
        let margin = try XCTUnwrap(shot.cropping(to: CGRect(x: shot.width / 50, y: shot.height * 4 / 5, width: 1, height: 1)))
        var rgba: [UInt8] = [0, 0, 0, 0]
        rgba.withUnsafeMutableBytes { bytes in
            let context = CGContext(data: bytes.baseAddress, width: 1, height: 1, bitsPerComponent: 8, bytesPerRow: 4,
                                    space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)
            context?.draw(margin, in: CGRect(x: 0, y: 0, width: 1, height: 1))
        }
        let luma = (299 * Int(rgba[0]) + 587 * Int(rgba[1]) + 114 * Int(rgba[2])) / 1000
        XCTAssertEqual(luma < 128, suffix == "dark", "the background does not match the requested appearance")
        app.segmentedControls["View"].buttons["Usage"].tap()
        XCTAssertTrue(app.staticTexts["5-hour usage limit"].appears(within: 5), app.debugDescription)
        try save("usage")
        app.segmentedControls["View"].buttons["Agents"].tap()
        app.buttons["New task"].tap()
        XCTAssertTrue(app.textFields["What should the agent do?"].appears(within: 5), app.debugDescription)
        try save("new-task")
        app.terminate()

        app = try launchDemo()
        app.open(try XCTUnwrap(URL(string: "collie://agent?m=demo&t=term_demo")))
        let latest = app.staticTexts.matching(NSPredicate(format: "label BEGINSWITH %@", "Will do.")).firstMatch
        XCTAssertTrue(latest.appears(within: 10), app.debugDescription)
        try save("chat")
        app.buttons["Terminal"].firstMatch.tap()
        XCTAssertTrue(app.scrollViews.firstMatch.appears(within: 5), app.debugDescription)
        try save("terminal")
        app.terminate()

        app = try launchDemo(["--approval-demo"])
        app.open(try XCTUnwrap(URL(string: "collie://agent?m=demo&t=term_2")))
        XCTAssertTrue(app.buttons["Approve"].appears(within: 10), app.debugDescription)
        try save("approval")
        app.terminate()
    }

    private func launchDemo(_ arguments: [String] = []) throws -> XCUIApplication {
        let app = XCUIApplication()
        let fixture = try XCTUnwrap(Bundle(for: Self.self).url(forResource: "screenshot-demo", withExtension: "txt"))
        app.launchArguments = ["-AppleLanguages", "(en)", "-AppleLocale", "en_US", "--terminal-demo", fixture.path] + arguments
        app.launchEnvironment["COLLIE_DEMO_SCREENSHOTS"] = "1"
        app.launch()
        XCTAssertTrue(app.buttons["New task"].appears(within: 10), app.debugDescription)
        return app
    }
}
