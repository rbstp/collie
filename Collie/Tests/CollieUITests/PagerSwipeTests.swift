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
