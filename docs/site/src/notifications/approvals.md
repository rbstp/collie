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

## Answering by hand

Approvals, questions and trust prompts from all three agents can also be answered with the terminal buttons, or by typing in the session composer. Typed answers are pasted into the current prompt and submitted with Enter. This does not ask for Face ID, and it is not tied to one approval: if the prompt changes, the new prompt receives your input.

## Done alerts

When an agent finishes a turn that took 30 seconds or more, you get one alert with its name and workspace, unless it is on screen. Tapping it opens the agent. Turn them off in **Settings > Notifications > Notify when an agent finishes**.

## What Apple can see

The visible part of an alert only says which agent is blocked and where. The command is encrypted end to end with a key generated on your phone, and decrypted by the app's notification extension. Apple still sees agent and workspace names, opaque IDs and timing. Details are in the [threat model](../reference/threat-model.md#apple-sees-push-payload-metadata).
