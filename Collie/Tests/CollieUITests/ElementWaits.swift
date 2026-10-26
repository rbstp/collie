import XCTest

/// Polls `condition` until it holds or `timeout` passes. Each try should read one query or one `Screen`: XCTest's own
/// waits (waitForExistence, wait(for:toEqual:)) snapshot the whole app for a debug description after every miss, and on a
/// loaded runner those snapshots took seconds each and ended in "Timed out while evaluating UI query".
@MainActor
func eventually(within timeout: TimeInterval = 10, _ condition: () -> Bool) -> Bool {
    let deadline = Date.now.addingTimeInterval(timeout)
    while !condition() {
        if Date.now >= deadline { return false }
        usleep(250_000)
    }
    return true
}

/// An element by type and predicate, found the same way by a main window query and in a `Screen`.
struct Match {
    let type: XCUIElement.ElementType
    let predicate: NSPredicate

    init(_ type: XCUIElement.ElementType, _ predicate: NSPredicate) {
        self.type = type
        self.predicate = predicate
    }

    /// The identifier, label or placeholder, as a query's subscript matches it.
    init(_ type: XCUIElement.ElementType, _ name: String) {
        self.init(type, NSPredicate(format: "identifier == %@ OR label == %@ OR placeholderValue == %@", name, name, name))
    }
}

/// The main window's elements from one snapshot. A query per element and attribute snapshots the app each time, and the
/// keyboard's windows with it.
struct Screen {
    let window: CGRect
    let elements: [any XCUIElementSnapshot]

    subscript(match: Match) -> (any XCUIElementSnapshot)? {
        elements.first { (match.type == .any || $0.elementType == match.type) && match.predicate.evaluate(with: $0) }
    }
}

@MainActor
extension XCUIElement {
    func appears(within timeout: TimeInterval) -> Bool {
        eventually(within: timeout) { exists }
    }

    func disappears(within timeout: TimeInterval) -> Bool {
        eventually(within: timeout) { !exists }
    }

    /// The frame once it holds still between two polls: rotations, paging, scrolls and the keyboard move it for a while.
    func settledFrame(within timeout: TimeInterval = 10) -> CGRect? {
        var last = CGRect.null
        let settled = eventually(within: timeout) {
            let frame = (try? snapshot())?.frame ?? .null
            defer { last = frame }
            return !frame.isEmpty && frame == last
        }
        return settled ? last : nil
    }
}

@MainActor
extension XCUIApplication {
    func screen() -> Screen? {
        guard let window = try? windows.firstMatch.snapshot() else { return nil }
        func flatten(_ element: any XCUIElementSnapshot) -> [any XCUIElementSnapshot] {
            [element] + element.children.flatMap(flatten)
        }
        return Screen(window: window.frame, elements: flatten(window))
    }

    func element(_ match: Match) -> XCUIElement {
        windows.firstMatch.descendants(matching: match.type).matching(match.predicate).firstMatch
    }

    /// Types into a focused field, clearing and typing again when the simulator drops keystrokes, and returns the field by
    /// its value, which a vertical prompt can surface as a text view.
    func type(_ text: String, into field: XCUIElement) -> XCUIElement {
        // The main window only: an app-wide query also walks the keyboard's windows, and it timed out on a loaded runner
        // while the keyboard was resyncing with the field.
        let typed = element(Match(.any, NSPredicate(format: "value == %@", text)))
        let clear = windows.firstMatch.buttons["Clear text"]
        if clear.exists { clear.tap() }
        field.typeText(text)
        for _ in 0..<3 where !typed.appears(within: 10) {
            if clear.exists { clear.tap() }
            field.typeText(text)
        }
        XCTAssertTrue(typed.appears(within: 10), debugDescription)
        return typed
    }

    // While a system menu is open, UIKit can keep a finished pacing animation on a hidden view, and XCTest's wait for
    // animations to end then runs out its 60 s after the event that opens the menu and again before the item tap.
    // XCTest's private interaction options skip those two waits: 1 skips the wait before an event, 2 skips both.
    func choose(_ item: XCUIElement, opening open: () -> Void) {
        withInteractionOptions(2, open)
        // The item exists before the menu has laid it out, and asking a frameless item for isHittable records a failure.
        XCTAssertTrue(eventually(within: 15) {
            guard let laidOut = try? item.snapshot().frame, !laidOut.isEmpty, frame.contains(laidOut) else { return false }
            return item.isHittable
        }, debugDescription)
        withInteractionOptions(1) { item.tap() }
    }

    /// Rotates and waits for the window to follow. A rotation with the keyboard up in a session could leave the app never
    /// idle for the rest of the test, and XCTest then waited 60 s after every rotation and tap, so the rotation skips that
    /// wait (see choose). Element frames lag the window, so callers wait on what they check next.
    @discardableResult
    func rotate(to orientation: UIDeviceOrientation) -> Bool {
        withInteractionOptions(2) { XCUIDevice.shared.orientation = orientation }
        let window = windows.firstMatch
        return eventually(within: 30) {
            guard let frame = try? window.snapshot().frame else { return false }
            return (frame.width > frame.height) == orientation.isLandscape
        }
    }

    func withInteractionOptions(_ options: UInt32, _ body: () -> Void) {
        let selector = NSSelectorFromString("_performWithInteractionOptions:block:")
        guard responds(to: selector) else { return body() }
        typealias Perform = @convention(c) (AnyObject, Selector, UInt32, @convention(block) () -> Void) -> Void
        withoutActuallyEscaping(body) { body in
            unsafeBitCast(method(for: selector), to: Perform.self)(self, selector, options, body)
        }
    }
}
