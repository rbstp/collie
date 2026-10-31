# Approvals and alerts

With [push notifications](../install/push.md) set up, `collied` alerts you when an agent needs you or finishes.

## Approval alerts

When an agent blocks on a permission prompt, a notification names the tool call: the exact command or file with the [Claude Code hook](../install/claude-code.md#the-permissionrequest-hook), otherwise what `collied` read from the screen. Tap it to open the session.

For a Claude Code permission prompt, choose **Approve** or **Deny** on the notification or in the app. On the notification, iOS first asks you to unlock a locked iPhone; in the app, every decision asks for Face ID or the passcode.

Claude Code can ask even in auto or bypass mode, for example before an `rm` on a path it cannot check. When it says why, such as "Dangerous rm operation on statically-unresolvable target: command substitution output", the notification shows that reason below the command, and so does the session's approval card, with a shield symbol.

Before it answers, `collied` reads the screen again. If the prompt changed, it refuses your answer as superseded rather than press it into the new prompt.

For Codex and Copilot CLI, the notification has no Approve or Deny: answer in the session. A Codex notification previews the pending question, the action or a recognizable assistant response; on an ambiguous screen, it shows `Blocked in <workspace>`, never your own input.

Once the agent moves on, `collied` tries to remove the notification.

<div class="screenshots">
<img class="screenshot-light" src="../images/approval-light.png" width="300" height="652" loading="lazy" alt="A blocked Claude Code agent with its approval card: the Bash command, its output, why Claude asked, and Approve, Approve always and Deny buttons">
<img class="screenshot-dark" src="../images/approval-dark.png" width="300" height="652" loading="lazy" alt="A blocked Claude Code agent with its approval card: the Bash command, its output, why Claude asked, and Approve, Approve always and Deny buttons">
</div>

## Answering by hand

You can also answer approvals, questions and trust prompts from all three agents with the terminal buttons, or by typing in the session composer, which pastes your answer into the current prompt and presses Enter. Answering by hand asks for no Face ID and is not tied to one approval: if the prompt changes, the new prompt receives your input.

## Done alerts

When an agent finishes a turn of 30 seconds or more, you get one alert with its name and workspace, unless the agent is on screen. Tap it to open the agent. To turn these alerts off, use **Settings > Notifications > Notify when an agent finishes**, which also covers the Woke and Archived alerts of [pull requests](../app/pull-requests.md).

## What Apple can see

Only your phone can read alert titles, bodies, names, pull request details and commands: `collied` encrypts them with a key your phone made for this machine. Apple receives generic text and ciphertext, plus opaque IDs, urgency, timing, frequency and payload size. If the phone cannot decrypt an alert in time, it shows generic text without approval controls. The [threat model](../reference/threat-model.md#apple-sees-push-payload-metadata) has the details.
