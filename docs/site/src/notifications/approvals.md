# Approvals and alerts

With [push notifications](../install/push.md) set up, `collied` alerts you when an agent needs you or finishes.

## Approval alerts

When an agent blocks on a permission prompt, a notification names the tool call: the exact command or file with the [Claude Code hook](../install/claude-code.md#the-permissionrequest-hook), otherwise what was read from the screen. Tapping it opens the matching session.

For Claude Code:

1. Approve or Deny from the notification, after unlocking the iPhone, or in the app.
2. Confirm with Face ID or the passcode. Every decision asks again.

`collied` re-reads the screen before it answers. If the prompt changed in the meantime, the answer is refused as superseded rather than pressed into the new prompt.

Codex and Copilot CLI send blocked notifications without Approve or Deny. Answer them in the session.

The notification is removed once the agent moves on, on a best-effort basis.

<div class="screenshots">
<img class="screenshot-light" src="../images/approval-light.png" width="300" height="652" loading="lazy" alt="A blocked Claude Code agent with its approval card: the Bash command, its output, and Approve, Approve always and Deny buttons">
<img class="screenshot-dark" src="../images/approval-dark.png" width="300" height="652" loading="lazy" alt="A blocked Claude Code agent with its approval card: the Bash command, its output, and Approve, Approve always and Deny buttons">
</div>

## Answering by hand

Approvals, questions and trust prompts from all three agents can also be answered with the terminal buttons, or by typing in the session composer. Typed answers are pasted into the current prompt and submitted with Enter. This does not ask for Face ID, and it is not tied to one approval: if the prompt changes, the new prompt receives your input.

## Done alerts

When an agent finishes a turn that took 30 seconds or more, you get one alert with its name and workspace, unless it is on screen. Tapping it opens the agent. Turn them off in **Settings > Notifications > Notify when an agent finishes**.

## What Apple can see

Alert titles, bodies, names, pull request details and commands are encrypted with your phone's per-machine key and decrypted locally by the notification extension. Apple receives generic text and ciphertext, plus opaque IDs, urgency, timing, frequency and payload size. A failed decryption or extension timeout leaves generic text without approval controls. Details are in the [threat model](../reference/threat-model.md#apple-sees-push-payload-metadata).
