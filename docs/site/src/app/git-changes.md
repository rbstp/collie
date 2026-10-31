# Git changes and feedback

To review what the agent changed, open the session's **More** menu and choose **Git changes**. It works only for a checkout inside the machine's task roots (`[tasks] roots` in `collied.toml`).

## Review the changes

When the repository has other worktrees, pick one in the **Branch** picker. Then choose a scope:

- **Uncommitted**: staged, unstaged and untracked files.
- **Branch**: every file the branch changed since it left its default branch (`origin/HEAD`, then `main`, then `master`).

Open a file to see its diff; **Wrap lines** turns phone-width wrapping on or off. Mark a file **Viewed** in its diff header to collapse it and move it last; this phone keeps the mark until the diff changes. Listings show up to 200 entries, and long diffs only a preview.

## Send feedback

1. Select text in a diff, choose **Feedback**, then write a comment. To comment on the whole file, tap the feedback button in its header.
2. Collect comments across files, editing them as needed.
3. Tap **Send feedback** to send them all to the agent at once.

Comments survive closing Git changes and a failed send, and sending leaves your composer alone. As with any prompt, feedback never goes to a blocked agent or replaces unsent text in its input box.
