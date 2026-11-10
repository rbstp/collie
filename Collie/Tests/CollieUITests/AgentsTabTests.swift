import XCTest

@MainActor
final class AgentsTabTests: XCTestCase {
    func testInboxSnoozesAndSearchesAndEveryLayoutSwipesToUsage() throws {
        continueAfterFailure = false
        let app = try demoApp("attachment-demo", ["--subagent-demo"])
        app.launch()
        defer { app.terminate() }
        let layout = app.buttons["Layout"]
        XCTAssertTrue(layout.appears(within: 10), app.debugDescription)
        let view = app.segmentedControls["View"]
        let agents = view.buttons["Agents"]
        let (color, frame) = (settledColor(agents), agents.frame)
        agents.tap()
        app.choose(app.buttons["Inbox"]) { layout.tap() }

        let row = app.staticTexts["Dark mode follows the system setting on every page."]
        XCTAssertTrue(row.appears(within: 5), app.debugDescription)
        let snooze = app.buttons["Snooze"]
        // A fixed distance: past about half the row, the drag is a full swipe that snoozes without showing the button.
        let start = row.coordinate(withNormalizedOffset: CGVector(dx: 0.1, dy: 0.5))
        start.press(forDuration: 0.1, thenDragTo: start.withOffset(CGVector(dx: 180, dy: 0)), withVelocity: .slow, thenHoldForDuration: 0)
        XCTAssertTrue(snooze.appears(within: 5), app.debugDescription)
        XCTAssertTrue(agents.isSelected, app.debugDescription)
        snooze.tap()
        let hour = app.buttons["1 hour"]
        XCTAssertTrue(hour.appears(within: 5), app.debugDescription)
        app.otherElements["PopoverDismissRegion"].firstMatch.tap()
        XCTAssertTrue(hour.disappears(within: 5), app.debugDescription)

        let filter = app.segmentedControls["Filter"].buttons.firstMatch
        XCTAssertNotNil(filter.settledFrame(), app.debugDescription)
        let filterRow = filter.frame
        app.buttons["Search"].tap()
        let search = app.textFields.matching(NSPredicate(format: "placeholderValue == %@", "Search agents")).firstMatch
        let keyboard = app.keyboards.firstMatch
        XCTAssertTrue(keyboard.appears(within: 10), app.debugDescription)
        XCTAssertNotNil(search.settledFrame(), app.debugDescription)
        let before = search.frame
        XCTAssertLessThan(abs(before.midY - filterRow.midY), 4, app.debugDescription)
        XCTAssertFalse(filter.exists, app.debugDescription)
        XCTAssertFalse(app.navigationBars.buttons["Search"].exists, app.debugDescription)
        for _ in 0..<3 { app.swipeUp() }
        let last = app.staticTexts["The rate limiter is merged."]
        XCTAssertNotNil(last.settledFrame(), app.debugDescription)
        XCTAssertEqual(search.frame, before, app.debugDescription)
        XCTAssertTrue(last.isHittable, app.debugDescription)
        XCTAssertLessThanOrEqual(last.frame.maxY, keyboard.frame.minY, app.debugDescription)
        app.buttons["Close search"].tap()
        XCTAssertTrue(filter.appears(within: 5), app.debugDescription)
        XCTAssertTrue(app.navigationBars.buttons["Search"].appears(within: 5), app.debugDescription)

        // The tapped selection kept its accent and size. Read here rather than right after the tap: a lost accent can show up late.
        XCTAssertEqual(agents.frame, frame)
        for (wanted, actual) in zip(color, settledColor(agents)) {
            XCTAssertEqual(actual, wanted, accuracy: 0.08)
        }

        // Back to layouts already shown too: the grid's page lock once left the pager unable to swipe after a switch.
        for (index, name) in ["Inbox", "Grid", "List", "Inbox", "Grid"].enumerated() {
            if index > 0 { app.choose(app.buttons[name]) { layout.tap() } }
            if index == 1 {
                // A working agent's card lists its first subagent and counts the rest, four once starred; a blocked agent's lists none.
                let card = app.buttons.containing(NSPredicate(format: "label CONTAINS %@", "fix the build")).firstMatch
                XCTAssertTrue(card.appears(within: 5), app.debugDescription)
                let label = card.label
                XCTAssertTrue(label.contains("Review approval flow") && label.contains("+4 more"), app.debugDescription)
                XCTAssertFalse(label.contains("Run approval tests"), app.debugDescription)
                let blocked = app.buttons.containing(NSPredicate(format: "label CONTAINS %@", "add approval tests")).firstMatch
                XCTAssertTrue(blocked.exists, app.debugDescription)
                XCTAssertFalse(blocked.label.contains("Review deny path"), app.debugDescription)
                app.choose(app.buttons["Star"]) { card.press(forDuration: 1) }
                XCTAssertTrue(eventually { card.label.contains("+1 more") }, app.debugDescription)
                let starred = card.label
                for title in ["Review approval flow", "Run approval tests", "Check retry handling", "Inspect test fixtures"] {
                    XCTAssertTrue(starred.contains(title), app.debugDescription)
                }
                XCTAssertFalse(starred.contains("Review documentation"), app.debugDescription)
            }
            XCTAssertTrue(agents.isSelected, app.debugDescription)
            if name == "Inbox" {
                view.buttons["Usage"].tap()
            } else {
                app.swipeLeft()
            }
            XCTAssertTrue(eventually { view.buttons["Usage"].isSelected }, "\(name)\n\(app.debugDescription)")
            // Usage has no layout or search, so their buttons hide rather than sit dimmed.
            XCTAssertTrue(eventually { !layout.exists && !app.buttons["Search"].exists }, "\(name)\n\(app.debugDescription)")
            XCTAssertTrue(app.buttons["New task"].exists, "\(name)\n\(app.debugDescription)")
            // On iPhone Pro Max the app's center lands on the Range picker, which keeps a drag that starts on it.
            let start = app.coordinate(withNormalizedOffset: CGVector(dx: 0.2, dy: 0.3))
            start.press(forDuration: 0.05, thenDragTo: start.withOffset(CGVector(dx: 300, dy: 0)), withVelocity: .fast, thenHoldForDuration: 0)
            XCTAssertTrue(eventually { agents.isSelected }, "\(name)\n\(app.debugDescription)")
            XCTAssertTrue(layout.appears(within: 5), "\(name)\n\(app.debugDescription)")
            XCTAssertEqual(app.buttons["Search"].exists, name == "Inbox", "\(name)\n\(app.debugDescription)")
        }
    }

    /// The label's color once two screenshots in a row agree: the selection animates.
    private func settledColor(_ element: XCUIElement) -> [Double] {
        var last: [Double] = []
        XCTAssertTrue(eventually {
            let color = (try? labelColor(element)) ?? []
            defer { last = color }
            return !color.isEmpty && color == last
        })
        return last
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
