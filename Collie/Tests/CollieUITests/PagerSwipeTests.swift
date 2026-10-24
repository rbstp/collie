import XCTest

@MainActor
final class PagerSwipeTests: XCTestCase {
    override func tearDown() async throws {
        XCUIDevice.shared.orientation = .portrait
    }

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
            app.choose(app.buttons[name]) { layout.tap() }
            XCTAssertTrue(view.buttons["Agents"].isSelected, app.debugDescription)
            if name == "Inbox" {
                view.buttons["Usage"].tap()
            } else {
                app.swipeLeft()
            }
            XCTAssertTrue(view.buttons["Usage"].wait(for: \.isSelected, toEqual: true, timeout: 5), "\(name)\n\(app.debugDescription)")
            // On iPhone Pro Max the app's center lands on the Range picker, which keeps a drag that starts on it.
            let start = app.coordinate(withNormalizedOffset: CGVector(dx: 0.2, dy: 0.3))
            start.press(forDuration: 0.05, thenDragTo: start.withOffset(CGVector(dx: 300, dy: 0)), withVelocity: .fast, thenHoldForDuration: 0)
            XCTAssertTrue(view.buttons["Agents"].wait(for: \.isSelected, toEqual: true, timeout: 5), "\(name)\n\(app.debugDescription)")
        }
    }

    func testAgentsPageStartsAtTheLeadingEdge() throws {
        let app = XCUIApplication()
        let fixture = try XCTUnwrap(Bundle(for: Self.self).url(forResource: "attachment-demo", withExtension: "txt"))
        app.launchArguments = ["-AppleLanguages", "(en)", "-AppleLocale", "en_US", "--terminal-demo", fixture.path]
        app.launch()
        defer { app.terminate() }
        XCTAssertTrue(app.segmentedControls["View"].buttons["Agents"].appears(within: 10), app.debugDescription)
        let page = app.collectionViews.firstMatch.cells.firstMatch
        XCTAssertTrue(page.appears(within: 5), app.debugDescription)
        XCTAssertEqual(page.frame.minX, 0, app.debugDescription)
    }

    func testUsageCardsKeepTheAgentsListMargins() throws {
        continueAfterFailure = false
        let app = XCUIApplication()
        let fixture = try XCTUnwrap(Bundle(for: Self.self).url(forResource: "attachment-demo", withExtension: "txt"))
        app.launchArguments = ["-AppleLanguages", "(en)", "-AppleLocale", "en_US", "--terminal-demo", fixture.path]
        app.launch()
        defer { app.terminate() }
        let layout = app.buttons["Layout"]
        XCTAssertTrue(layout.appears(within: 10), app.debugDescription)
        app.choose(app.buttons["List"]) { layout.tap() }
        let view = app.segmentedControls["View"]
        // The lists' cells, not the pager's pages.
        let rows = app.collectionViews.firstMatch.collectionViews.cells
        let row = rows.containing(NSPredicate(format: "label CONTAINS %@", "fix the build")).firstMatch
        let card = rows.containing(NSPredicate(format: "label == %@", "5-hour usage limit")).firstMatch
        view.buttons["Usage"].tap()
        // Each rotation happens with Usage on screen. iPhone Duo's cover display does not rotate, so there every pass is portrait.
        for orientation: UIDeviceOrientation in [.portrait, .landscapeLeft, .portrait] {
            XCUIDevice.shared.orientation = orientation
            let window = app.windows.firstMatch
            for _ in 0..<10 where (window.frame.width > window.frame.height) != orientation.isLandscape { usleep(500_000) }
            let usage = settledFrame(of: card, app)
            view.buttons["Agents"].tap()
            let agents = settledFrame(of: row, app)
            XCTAssertEqual(usage.minX, agents.minX, "\(orientation.rawValue)\n\(app.debugDescription)")
            XCTAssertEqual(usage.maxX, agents.maxX, "\(orientation.rawValue)\n\(app.debugDescription)")
            view.buttons["Usage"].tap()
        }
    }

    /// The frame once it holds still: rotation and paging move it for a while.
    private func settledFrame(of element: XCUIElement, _ app: XCUIApplication) -> CGRect {
        XCTAssertTrue(element.appears(within: 5), app.debugDescription)
        var frame = element.frame
        for _ in 0..<20 {
            usleep(500_000)
            let now = element.frame
            if now == frame { break }
            frame = now
        }
        return frame
    }

    func testTappedSelectionKeepsItsColor() throws {
        let app = XCUIApplication()
        let fixture = try XCTUnwrap(Bundle(for: Self.self).url(forResource: "attachment-demo", withExtension: "txt"))
        app.launchArguments = ["-AppleLanguages", "(en)", "-AppleLocale", "en_US", "--terminal-demo", fixture.path]
        app.launch()
        defer { app.terminate() }
        let agents = app.segmentedControls["View"].buttons["Agents"]
        XCTAssertTrue(agents.appears(within: 10), app.debugDescription)
        sleep(1)
        let before = try labelColor(agents)
        let frame = agents.frame
        agents.tap()
        sleep(2)
        let after = try labelColor(agents)
        XCTAssertEqual(agents.frame, frame)
        for (wanted, actual) in zip(before, after) {
            XCTAssertEqual(actual, wanted, accuracy: 0.08)
        }
    }

    /// The most saturated pixel inside the element: its accent-colored label.
    private func labelColor(_ element: XCUIElement) throws -> [Double] {
        let image = XCUIScreen.main.screenshot().image
        let frame = element.frame
        let crop = try XCTUnwrap(image.cgImage?.cropping(to: CGRect(
            x: frame.minX * image.scale, y: frame.minY * image.scale,
            width: frame.width * image.scale, height: frame.height * image.scale
        )))
        var pixels = [UInt8](repeating: 0, count: crop.width * crop.height * 4)
        try pixels.withUnsafeMutableBytes { bytes in
            let context = try XCTUnwrap(CGContext(
                data: bytes.baseAddress, width: crop.width, height: crop.height, bitsPerComponent: 8, bytesPerRow: crop.width * 4,
                space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
            ))
            context.draw(crop, in: CGRect(x: 0, y: 0, width: crop.width, height: crop.height))
        }
        let pixel = stride(from: 0, to: pixels.count, by: 4).max { a, b in
            func saturation(_ i: Int) -> Int { Int(pixels[i..<i + 3].max()!) - Int(pixels[i..<i + 3].min()!) }
            return saturation(a) < saturation(b)
        } ?? 0
        return pixels[pixel..<pixel + 3].map { Double($0) / 255 }
    }
}
