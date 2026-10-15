# Agents

The **Agents** tab lists every herdr agent on every paired machine, grouped by machine with blocked agents first. Each agent shows its status: `idle`, `working`, `blocked` or `done`. A machine that is asleep or off shows as offline without slowing the others.

Pull down to retry every machine, or tap a machine's header to retry that one.

## Layouts

The Agents tab has three layouts.

### Grid

The default. Each card is a live preview of the agent's screen. Claude Code, Codex and Copilot CLI cards keep the agent's output and drop its input box and status lines; a blocked agent's card shows its whole screen.

- **Star** a card with a long press. Starred cards lead their machine's section at twice the height. Stars are kept on the machine, shared by your phones, until the pane closes or the machine is removed.
- **Reorder** by dragging a card to an insertion line. Moving a card between the starred and unstarred groups also stars or unstars it. The order is saved on this phone.

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

## Agent actions

Long-press an agent, or use the agent screen's menu, to:

- follow it on the Lock Screen (see [Live Activities](../notifications/live-activities.md));
- close its pane or workspace;
- open its [Git changes](git-changes.md).

**Focus on** followed by the machine name brings the agent's pane to the front in herdr.

## Appearance

**Settings > Appearance > Accent color** offers Blue (the default), Orange, Green, Red and Dark Gold. The choice stays on this phone. Destructive actions stay red and status colors keep their meaning.
