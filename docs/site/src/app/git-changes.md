# Git changes and feedback

Open a session's **More** menu and choose **Git changes** to review what the agent changed in its checkout.

## Pick the checkout

When the repository has other worktrees, such as one the agent created for its branch, pick the one to review in the **Branch** picker. The session keeps your choice.

## Two views

- **Uncommitted**: staged, unstaged and untracked files, listed separately with additions and deletions.
- **Branch**: every file the branch changed since it left its default branch (`origin/HEAD`, then `main`, then `master`), named at the top.

Mark a file **Viewed** from its diff header to collapse it and move it after the others. This phone keeps the mark until the file's diff changes.

## Read a diff

Open a file to see its diff, with old and new line numbers and tinted additions and deletions. **Wrap lines** switches between phone-width wrapping and full lines that scroll sideways. Binary and empty files get views that explain them.

Listings show up to 200 entries. Long diffs show only a preview, and untracked file previews stop at 1 MiB.

## Send feedback

1. Select text in a diff, choose **Feedback** next to Copy, then write a comment. It appears below the selected lines and quotes only the selected code. To comment on the whole file, use the feedback button in the file header.
2. Collect comments across files and edit them as needed.
3. Tap **Send feedback** to send them all to the agent at once.

Sending feedback leaves your composer's prompt and attachments alone. Your comments survive closing and reopening Git changes, and a failed send. When a diff changes on refresh, existing comments stay below it with their original quotes. As with any prompt, feedback never goes to a blocked agent and never replaces unsent text in the agent's input box.

> [!NOTE]
> Git changes works only for a checkout inside the machine's task roots (`[tasks] roots` in `collied.toml`).
