# Git changes and feedback

To review what the agent changed, open the session's **More** menu and choose **Git changes**. It works only for a checkout inside the machine's task roots (`[tasks] roots` in `collied.toml`).

## Pick a worktree

Git changes opens on the repository's worktrees. Each row shows the branch, the last commit and how long ago it was made, commits ahead of the default branch, files not yet committed, lines added and removed, and its open pull request's number and state. A location arrow marks the session's own checkout, and colored dots count the agents working in that worktree.

Below them, **Pull requests without a worktree** lists your open pull requests whose worktree is gone, so you can still review them.

## Clean up worktrees

1. Tap **Select**, then pick the worktrees you no longer need. The agent and changes markers stay on the rows to help you choose. You cannot pick the session's own checkout or a `main` or `master` worktree.
2. Tap **Clean up** to send the agent one prompt naming each picked worktree and its branch.

The prompt asks the agent to check that each worktree is not in use: no agent or process works in it, it has no uncommitted, untracked or unpushed work, and its pull request is merged or closed, or it has none. It asks the agent to remove only the worktrees that pass, with their local branches, never the main worktree or the default branch, and to tell you which it kept and why. If the agent's checkout changed since the list loaded, refresh it before cleaning up. Like feedback, the prompt never goes to a blocked agent or replaces unsent text in its input box. Leaving Git changes ends Select mode.

## Review the changes

Open a worktree to see its folder, the agents working in it (tap one to open its session) and its files. Then choose a scope:

- **Uncommitted**: staged, unstaged and untracked files.
- **Branch**: every file the branch changed since it left its default branch (`origin/HEAD`, then `main`, then `master`).
- **Pull request**: while the branch has an open or draft pull request, its files and diffs exactly as GitHub shows them, read with `gh` on the machine.

Each file shows its status letter, path and lines added and removed. Open a file to see its diff, a compact unified view like GitHub's: a one-line header with the path and its added and removed line counts, then each line with its old and new line numbers, added lines tinted green and removed lines red. **Wrap lines** turns phone-width wrapping on or off. Mark a file **Viewed** in its diff header to collapse it and move it last; this phone keeps the mark until the diff changes. Listings show up to 200 entries, and long diffs only a preview.

## Send feedback

1. Select text in a diff, in any scope, choose **Feedback**, then write a comment. To comment on the whole file, tap the feedback button in its header.
2. Collect comments across files, editing them as needed.
3. Tap **Send feedback** to send them all to the agent at once.

Comments survive closing Git changes and a failed send, and sending leaves your composer alone. As with any prompt, feedback never goes to a blocked agent or replaces unsent text in its input box.
