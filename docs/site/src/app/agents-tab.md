# Agents

The **Agents** tab lists every herdr agent on every paired machine, grouped by machine with blocked agents first. Each agent shows its status: `idle`, `working`, `blocked` or `done`. A machine that is asleep or off shows as offline without slowing the others.

Pull down to retry every machine, or tap a machine's header to retry that one.

<div class="screenshots">
<img class="screenshot-light" src="../images/agents-light.png" width="300" height="652" loading="lazy" alt="The Agents tab: a grid of six agents in three workspaces on one Mac, each card a preview of the agent's screen, one blocked on an approval">
<img class="screenshot-dark" src="../images/agents-dark.png" width="300" height="652" loading="lazy" alt="The Agents tab: a grid of six agents in three workspaces on one Mac, each card a preview of the agent's screen, one blocked on an approval">
</div>

## Machine order

Open **Machines**, tap **Edit**, then drag the reorder handles to arrange your machines. Tap **Done** to finish. The order stays on this phone across app restarts and applies to the grouped Agents layouts, Usage and navigation between agents on different machines. Offline machines keep their positions, newly paired machines are added at the end, and removing a machine removes it from the order.

Each machine keeps its existing agent order. Inbox continues to rank agents by status and priority across machines.

## Layouts

The Agents tab has three layouts.

### Grid

The default. Each card is a live preview of the agent's screen. Claude Code, Codex and Copilot CLI cards keep the agent's output and drop its input box and status lines; a blocked agent's card shows its whole screen.

Running subagents appear on glass at the top of the preview, with each title and elapsed time, updated every minute. A normal card shows up to two subagents and a starred card up to four, followed by "+N more" when needed. The latest output stays visible below the list; previews grow at accessibility text sizes to leave room for both. The list disappears when the subagents finish and is hidden while the agent is blocked, keeping its prompt visible. VoiceOver includes the subagents when reading the card.

- **Star** a card with a long press. Starred cards lead their machine's section at twice the height. Stars are kept on the machine, shared by your phones, until the pane closes or the machine is removed.
- **Reorder** by dragging a card to an insertion line. Moving a card between the starred and unstarred groups also stars or unstars it. The order is saved on this phone.

Card positions stay fixed through status updates. Drop a card above or below a starred card, or on any edge of an unstarred card in a grid with several rows. A single row only offers left and right, and a single column above and below. When the other group is empty, a glass drop area appears during the drag: above the cards to star, below them to unstar. The Agents page stays in place until the drag ends.

### List

A compact list of agents, with their status as icons.

### Inbox

Search agents across machines and filter them:

- **Needs attention**: blocked agents and recent completions.
- **Working**: agents that are working, including blocked ones.
- **Inactive**: idle agents and completions older than 24 hours.

Each row shows the latest reply, your last prompt, the workspace, agent kind, machine and elapsed time.

Swipe a row to the right and choose **Snooze** to hide it for 1 hour, 4 hours, until 8:00 the next morning or until a time you pick. A snoozed agent comes back early when it blocks or finishes another turn. Blocked agents cannot be snoozed. Snoozed agents wait in a collapsed **Snoozed** section; long-press one and choose **Wake now** to bring it back. Snoozes are kept on this phone.

## Remaining context

A small ring shows how much context a Claude Code or Codex agent has left, read from its transcript on the machine.

## Running subagents

Claude Code, GitHub Copilot CLI and Codex report their running subagents with a title and elapsed time. Finished or interrupted work disappears; a follow-up starts a new elapsed time for the same child.

Copilot CLI uses the child names in the current session's local event log. Codex uses child thread metadata and turn events, not the text of the parent's reply. Unrelated sessions, idle children and sessions without a live provider lock are excluded. A large event log may take several refreshes to catch up before children appear. Older Codex forks without an explicit inherited-history boundary are not reported.

## Agent actions

Long-press an agent, or use the agent screen's menu, to:

- follow it on the Lock Screen (see [Live Activities](../notifications/live-activities.md));
- close its pane or workspace;
- open its [Git changes](git-changes.md).

**Focus on** followed by the machine name brings the agent's pane to the front in herdr.

## Appearance

**Settings > Appearance > Accent color** offers Blue (the default), Orange, Green, Red and Dark Gold. The choice stays on this phone. Destructive actions stay red and status colors keep their meaning.
