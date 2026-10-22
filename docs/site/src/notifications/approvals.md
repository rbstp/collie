# Approvals and alerts

With [push notifications](../install/push.md) set up, `collied` alerts you when an agent needs you or finishes.

## Approval alerts

When an agent blocks on a permission prompt, a notification names the tool call. With the [Claude Code hook](../install/claude-code.md#the-permissionrequest-hook), it names the exact command or file; otherwise, it shows what `collied` read from the screen. Tapping the notification opens the matching session.

For Claude Code:

1. Approve or Deny from the notification, after unlocking the iPhone, or in the app.
2. Confirm with Face ID or the passcode. Every decision asks again.

Claude Code can stop to ask even in auto or bypass mode, for example before an `rm` on a path it cannot check. When it says why, such as "Dangerous rm operation on statically-unresolvable target: command substitution output", the notification shows that reason above the command, and the approval card in the session shows it below the command with a shield symbol.

`collied` reads the screen again before it answers. If the prompt changed in the meantime, `collied` refuses your answer as superseded rather than press it into the new prompt.

For Codex and Copilot CLI, the blocked notification has no Approve or Deny: answer it in the session. A Codex notification previews the pending question, the action or a recognizable assistant response. When the screen is ambiguous, it shows `Blocked in <workspace>` rather than your own input.

Once the agent moves on, `collied` tries to remove the notification.

<div class="screenshots">
<img class="screenshot-light" src="../images/approval-light.png" width="300" height="652" loading="lazy" alt="A blocked Claude Code agent with its approval card: the Bash command, its output, why Claude asked, and Approve, Approve always and Deny buttons">
<img class="screenshot-dark" src="../images/approval-dark.png" width="300" height="652" loading="lazy" alt="A blocked Claude Code agent with its approval card: the Bash command, its output, why Claude asked, and Approve, Approve always and Deny buttons">
</div>

## Answering by hand

You can also answer approvals, questions and trust prompts from all three agents with the terminal buttons, or by typing in the session composer. The phone pastes a typed answer into the current prompt and submits it with Enter. Answering by hand does not ask for Face ID, and it is not tied to one approval: if the prompt changes, the new prompt receives your input.

## Done alerts

When an agent finishes a turn that took 30 seconds or more, you get one alert with its name and workspace, unless the agent is on screen. Tapping the alert opens the agent. To turn these alerts off, use **Settings > Notifications > Notify when an agent finishes**.

## What Apple can see

Only your phone can read alert titles, bodies, names, pull request details and commands: `collied` encrypts them with a key your phone made for this machine, and the phone decrypts them locally. Apple receives generic text and ciphertext, plus opaque IDs, urgency, timing, frequency and payload size. If the phone cannot decrypt an alert in time, it shows generic text without approval controls. The [threat model](../reference/threat-model.md#apple-sees-push-payload-metadata) has the details.
