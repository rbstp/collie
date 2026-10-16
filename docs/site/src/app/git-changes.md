# Git changes and feedback

Open a session's **More** menu and choose **Git changes** to review what the agent changed in its checkout.

## Pick the checkout

When the repository has other worktrees, such as one the agent created for its branch, pick it under **Branch**. The choice stays for that session.

## Two views

- **Uncommitted**: staged, unstaged and untracked files, listed separately with additions and deletions.
- **Branch**: every file the branch changed since it left its default branch (`origin/HEAD`, then `main`, then `master`), named at the top.

Mark a file **Viewed** from its diff header: it collapses and moves after the others, and the mark stays on this phone until the file's diff changes.

## Read a diff

Open a file for a diff with old and new line numbers and tinted additions and deletions. **Wrap lines** switches between phone-width wrapping and full lines with horizontal scrolling. Binary and empty files get their own explanatory views.

Listings show up to 200 entries, diffs are previewed up to a limit, and untracked file previews stop at 1 MiB.

## Send feedback

1. Select text in a diff and choose **Feedback** next to Copy. Write a comment; it appears below the selected lines, quoting only the selected code. The file header's feedback button adds a comment about the whole file.
2. Collect comments across files and edit them as needed.
3. Tap **Send feedback** to send them all to the same agent at once.

Your composer's prompt and attachments are left alone. Feedback survives closing and reopening Git changes, and a failed send. If a refreshed diff changes, existing comments stay below the diff with their original quotes. Sending respects approval safeguards and refuses to replace unsent text in the agent's input box.

> [!NOTE]
> Git changes needs an updated `collied`, and the checkout must be inside the machine's task roots (`[tasks] roots` in `collied.toml`).
