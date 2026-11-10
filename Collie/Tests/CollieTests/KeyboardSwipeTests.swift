import Testing
import UIKit

@testable import Collie

@MainActor
@Test func aSwipeDownDismissesOnlyADraftAtItsTop() {
    let draft = UITextView(frame: CGRect(x: 0, y: 0, width: 300, height: 100))
    draft.text = (1...12).map { "Line \($0) of the draft left in Claude Code on the Mac" }.joined(separator: "\n")
    draft.layoutIfNeeded()
    #expect(draft.contentSize.height > draft.bounds.height)
    #expect(KeyboardSwipe.dismisses(from: draft))
    draft.contentOffset.y = 1
    #expect(KeyboardSwipe.dismisses(from: draft), "a point of rubber band still counts as the top")
    draft.contentOffset.y = 40
    #expect(!KeyboardSwipe.dismisses(from: draft), "scrolled below its top, the swipe scrolls the draft back up")
    draft.contentInset.top = 20
    draft.contentOffset.y = -20
    #expect(KeyboardSwipe.dismisses(from: draft), "the top counts the inset")
    draft.contentOffset.y = 0
    #expect(!KeyboardSwipe.dismisses(from: draft))
    #expect(KeyboardSwipe.dismisses(from: UITextField()), "a one-line field never scrolls")
}
