# Agents

The **Agents** tab lists the herdr agents on every paired machine. Each agent shows its status: `idle`, `working`, `blocked` or `done`. A machine that is asleep or off shows as offline without slowing down the others.

Pull down to retry every machine, or tap a machine's header to retry that one.

<div class="screenshots">
<img class="screenshot-light" src="../images/agents-light.png" width="300" height="652" loading="lazy" alt="The Agents tab: a grid of six agents in three workspaces on one Mac, each card a preview of the agent's screen, one blocked on an approval">
<img class="screenshot-dark" src="../images/agents-dark.png" width="300" height="652" loading="lazy" alt="The Agents tab: a grid of six agents in three workspaces on one Mac, each card a preview of the agent's screen, one blocked on an approval">
</div>

## Machine order

To arrange your machines, open **Machines**, tap **Edit**, drag the handles, then tap **Done**. Grid, List and Usage follow this order, as does swiping between agents opened from Grid or List; Inbox sorts by status instead. This phone keeps the order, and newly paired machines go to the end.

## Layouts

The layout button at the top switches between three layouts.

### Grid

The default: each machine gets a section of cards. While **Settings > Sessions > Default view** is Chat (the default), each card previews the conversation: your last two messages and the agent's last two replies, without tool calls. Set to Terminal, or when Chat is not available for the agent or `collied` on its machine needs an update, a card previews the agent's screen without Claude Code's, Codex's or Copilot CLI's input box and status lines. A blocked agent's card always shows the whole screen, prompt included. The grid picks up a change to the setting the next time you open the tab.

[Running subagents](#running-subagents) appear at the top of the preview with their titles and elapsed times: up to two on a card and four on a starred card, then "+N more". While the agent is blocked, the card hides them so the prompt stays visible.

- **Star** a card: long-press it and choose **Star**. Starred cards lead their machine's section at twice the height. The machine keeps stars for all your phones until the pane closes or you remove the machine.
- **Reorder** cards by dragging. Dropping a card among the starred cards stars it, and among the others unstars it; when one group is empty, drop on the **Drop here to star** or **Drop here to unstar** area that appears. This phone keeps the order, and cards stay in place when statuses change.

### List

A compact list of agents grouped by machine, blocked first, with their status as icons.

### Inbox

One list of the agents on every machine, grouped by status with blocked agents first. Search it with the field at the top (swipe down on the field to hide the keyboard), or filter it:

- **Needs attention**: blocked agents and completions you have not opened.
- **Working**: working and blocked agents.
- **Inactive**: idle agents and completions older than 24 hours.

Each row shows the latest reply, your last prompt, the workspace, agent kind, machine and elapsed time.

To hide a row, swipe it right and choose **Snooze**: **1 hour**, **4 hours**, **Today 8:00** or **Tomorrow 8:00** (whichever comes next), or **Custom**. You cannot snooze a blocked agent, and a snoozed agent comes back early when it blocks or finishes another turn. Snoozed agents wait in a collapsed **Snoozed** section; long-press one and choose **Wake now** to bring it back. This phone keeps your snoozes.

## Remaining context

A small ring shows how much context a Claude Code or Codex agent has left, read from its transcript on the machine. Codex needs [herdr's hook](../agents/codex.md#set-up). For Claude Code, the ring counts down to the point where Claude Code compacts the conversation, like its own "until auto-compact" indicator, and shows 0% once the session passes it. With auto-compact off in Claude Code's `/config`, the ring counts down to the full context window.

## Running subagents

Claude Code, GitHub Copilot CLI and Codex agents show their running subagents, each with a title and elapsed time; Codex needs [herdr's hook](../agents/codex.md#set-up). A subagent disappears when it finishes or is interrupted, and a follow-up restarts its elapsed time. After Copilot's `/new`, Collie follows the new conversation in the same pane; when it cannot tell which conversation the pane shows, it does not guess.

Collie leaves out idle subagents, subagents of other sessions, a subagent that never writes its identifying metadata, and subagents forked by older Codex versions that do not mark where the inherited history ends. In a session with a large event log, subagents can take a few refreshes to appear.

Subagents also hold back [archiving when the pull request merges](pull-requests.md#archive-when-the-pull-request-merges): Collie waits while a subagent runs, while it cannot read a known subagent's data, and while it cannot tell which conversation a Copilot pane shows. Background work a Copilot pane holds from an earlier conversation also blocks archiving, though it does not appear. Unused Codex threads do not count.

## Agent actions

Long-press an agent to follow it on the Lock Screen (see [Live Activities](../notifications/live-activities.md)), close its pane or workspace, or [archive](new-task.md#archive) it. A session's **More** menu offers the same actions, plus [Git changes](git-changes.md), **Refresh**, and **Focus on** followed by the machine name, which brings the agent's pane to the front in herdr on that machine.

## Appearance

Choose an accent color in **Settings > Appearance > Accent color**: None, Blue (the default), Orange, Green, Red or Dark Gold. None uses the system text color, black or white, for the highest contrast; switches stay green. The choice applies to this phone only. Destructive actions stay red, and status colors keep their meaning.
