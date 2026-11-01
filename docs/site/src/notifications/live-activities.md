# Live Activities

Follow an agent to keep it on the Lock Screen and in the Dynamic Island: open the agent screen's menu, or long-press the agent, and choose **Follow on Lock Screen**. Following is off by default. You can follow up to 5 agents at once; each shows its status and for how long.

When a followed Claude Code agent blocks on a permission prompt, the activity shows the command with **Approve** and **Deny**, which need the iPhone unlocked. Other prompts, and Codex or Copilot CLI agents, arrive as a regular notification instead.

Through the publisher relay, an activity updates at most once a minute, but this limit never delays an approval or the activity's end.

Everything an activity shows, including names, status and pending commands, is encrypted so that only your phone can read it. Apple sees generic alert text, opaque IDs, priority, timing, size and each activity's lifecycle. Before the first unlock after a restart, or if the phone cannot decrypt an update, the activity shows a neutral state without approval controls.
