import CollieCore
import SwiftUI
import Testing
import UIKit
@testable import Collie

@Test func gitDiffRowsHideMetadataAndKeepCodeThatLooksLikeHeaders() {
    let patch = "diff --git a/a b/a\nindex 123..456 100644\n--- a/a\n+++ b/a\n@@ -9,3 +9,4 @@ section\n same\n---old\n+++new\n+🐑\n last\n\\ No newline at end of file\n@@ -30 +31 @@\n-before\n+after\n"
    let rows = GitDiffRow.parse(patch)
    #expect(rows.map(\.kind) == [.hunk, .context, .removed, .added, .added, .context, .notice, .hunk, .removed, .added])
    #expect(rows.map(\.oldLine) == [nil, 9, 10, nil, nil, 11, nil, nil, 30, nil])
    #expect(rows.map(\.newLine) == [nil, 9, nil, 10, 11, 12, nil, nil, nil, 31])
    #expect(rows[2].text == "--old")
    #expect(rows[3].text == "++new")
    #expect(rows[4].text == "🐑")
    #expect(rows[7].text == "@@ -30 +31 @@")
}

@Test func gitDiffHandlesAddedDeletedEmptyAndBinaryFiles() {
    let added = GitDiffRow.parse("@@ -0,0 +1,2 @@\n+\n+new\n")
    #expect(added.map(\.newLine) == [nil, 1, 2])
    #expect(added.allSatisfy { $0.oldLine == nil })
    #expect(added[1].text.isEmpty)
    let removed = GitDiffRow.parse("@@ -1 +0,0 @@\n-old\n")
    #expect(removed.map(\.oldLine) == [nil, 1])
    #expect(removed.allSatisfy { $0.newLine == nil })
    #expect(GitDiffRow.parse("diff --git a/a b/a\nnew file mode 100644\nindex 000..123\n").isEmpty)
    #expect(GitDiffRow.parse("New empty file.\n").first?.kind == .notice)
    #expect(GitDiffRow.parse("diff --git a/a b/a\nBinary files a/a and b/a differ\n").first?.kind == .notice)
    #expect(GitDiffRow.parse("@@ - + @@\n").isEmpty)
}

@Test func gitDiffKeepsAnEmptyContextLineWithoutAddingTheTrailingNewline() {
    let rows = GitDiffRow.parse("@@ -1,3 +1,3 @@\n a\n\n-b\n+c\n")
    #expect(rows.map(\.kind) == [.hunk, .context, .context, .removed, .added])
    #expect(rows.map(\.oldLine) == [nil, 1, 2, 3, nil])
    #expect(rows.map(\.newLine) == [nil, 1, 2, nil, 3])
    #expect(rows[2].text.isEmpty)
    #expect(GitDiffRow.parse("@@ -1,2 +1,2 @@\n a\n-b\n+c\n\n").count == 4)
}

@Test func gitDiffShowsCombinedMergeConflictDiffs() {
    let patch = """
        diff --cc f
        index b8cdd60,d24f24b..0000000
        --- a/f
        +++ b/f
        @@@ -1,6 -1,5 +1,11 @@@
          one
        ++<<<<<<< HEAD
         +TWO main
        ++||||||| c6c83ac
        ++two
        ++=======
        + TWO side
        ++>>>>>>> side
          \("")
        --three
          four
         +five

        """
    let rows = GitDiffRow.parse(patch)
    #expect(rows.map(\.kind) == [.hunk, .context, .added, .added, .added, .added, .added, .added, .added, .context, .removed, .context, .added])
    #expect(rows.map(\.newLine) == [nil, 1, 2, 3, 4, 5, 6, 7, 8, 9, nil, 10, 11])
    #expect(rows.allSatisfy { $0.oldLine == nil })
    #expect(rows.map(\.text) == ["@@@ -1,6 -1,5 +1,11 @@@", "one", "<<<<<<< HEAD", "TWO main", "||||||| c6c83ac", "two", "=======", "TWO side", ">>>>>>> side", "", "three", "four", "five"])
    let removed = GitDiffRow.parse("@@@ -1,3 -1,3 +1,2 @@@\n- a\n -b\n  c\n\n")
    #expect(removed.map(\.kind) == [.hunk, .removed, .removed, .context, .context])
    #expect(removed.map(\.newLine) == [nil, nil, nil, 1, 2])
}

@Test func gitDiffSelectionUsesUTF16AndAnchorsTheLastSelectedRow() throws {
    let rows = GitDiffRow.parse("@@ -1,2 +1,2 @@\n-old 🐑\n+new 🐑\n context\n")
    let block = GitDiffBlock(rows: rows)
    let selected = "old 🐑\nnew 🐑"
    let range = (block.text as NSString).range(of: selected)
    #expect(GitFeedback.selectedText(in: block.text, range: range) == selected)
    #expect(block.selectedRow(range) == 2)
    #expect(block.selectedRow((block.text as NSString).range(of: "old 🐑\n")) == 1)
    #expect(block.selectedRow(NSRange(location: NSNotFound, length: 2)) == nil)
    #expect(block.selectedRow(NSRange(location: 0, length: 0)) == nil)
    let blocks = GitDiffBlock.split(rows, after: [1, 2])
    #expect(blocks.map(\.lastRow) == [1, 2, 3])
    #expect(blocks.flatMap(\.rows) == rows)
    #expect(blocks.map(\.text).joined() == block.text)
    #expect(GitDiffBlock.split([], after: [0]).isEmpty)
}

@MainActor
@Test func gitDiffMenuKeepsCopyAndOffersFeedbackOnlyForASelection() throws {
    let block = GitDiffBlock(rows: GitDiffRow.parse("@@ -1 +1 @@\n-old\n+new 🐑\n"))
    let coordinator = GitDiffText.Coordinator(block: block, feedback: { _, _ in })
    let copy = UIAction(title: "Copy") { _ in }
    let view = UITextView()
    let range = (block.text as NSString).range(of: "new 🐑")
    let menu = try #require(coordinator.textView(view, editMenuForTextIn: range, suggestedActions: [copy]))
    #expect(menu.children.map(\.title) == ["Feedback", "Copy"])
    #expect(coordinator.textView(view, editMenuForTextIn: NSRange(location: 0, length: 0), suggestedActions: [copy]) == nil)
    coordinator.feedback = nil
    #expect(coordinator.textView(view, editMenuForTextIn: range, suggestedActions: [copy]) == nil)
}

@MainActor
@Test func gitReviewPreservesInlineAnchorsAndOriginalQuotes() throws {
    let review = GitReview()
    let file = GitFile(path: "file.swift", oldPath: nil, section: "unstaged", status: "M", additions: 1, deletions: 1)
    let patch = "@@ -1 +1 @@\n-old\n+new\n"
    let anchor = GitDiffAnchor(patch: patch, row: 2)
    let id = try #require(review.add(root: "/worktree", file: file, selection: "new", anchor: anchor))
    review.text(for: id).wrappedValue = "Explain this change."
    let comment = try #require(review.comments.first)
    #expect(comment.anchor == anchor)
    #expect(comment.anchor != GitDiffAnchor(patch: patch + " context\n", row: 2))
    #expect(comment.message == "Feedback for file.swift: \n\n```diff\nnew\n```\n\nExplain this change.")
}
