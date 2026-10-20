import UIKit

/// A downward swipe that starts on the focused text input dismisses the keyboard, for every field in the window.
@MainActor
final class KeyboardSwipe: NSObject, UIGestureRecognizerDelegate {
    static let shared = KeyboardSwipe()

    func install(on window: UIWindow) {
        guard !(window.gestureRecognizers ?? []).contains(where: { $0.delegate === self }) else { return }
        let swipe = UISwipeGestureRecognizer(target: self, action: #selector(swiped))
        swipe.direction = .down
        swipe.cancelsTouchesInView = false
        swipe.delegate = self
        window.addGestureRecognizer(swipe)
    }

    @objc private func swiped(_ swipe: UISwipeGestureRecognizer) {
        swipe.view?.endEditing(true)
    }

    func gestureRecognizer(_ gestureRecognizer: UIGestureRecognizer, shouldReceive touch: UITouch) -> Bool {
        guard let view = touch.view,
              let input = sequence(first: view, next: \.superview).first(where: { $0 is UITextInput && $0.isFirstResponder })
        else { return false }
        // A long draft scrolled below its top scrolls back up instead.
        guard let text = input as? UIScrollView else { return true }
        return text.contentOffset.y <= -text.adjustedContentInset.top + 1
    }

    func gestureRecognizer(_ gestureRecognizer: UIGestureRecognizer, shouldRecognizeSimultaneouslyWith other: UIGestureRecognizer) -> Bool {
        true
    }
}
