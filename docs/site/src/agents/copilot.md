# Run Copilot CLI from your phone

You can follow GitHub Copilot CLI agents, read them in Chat and work with them in Terminal.

## What works

| Feature | Copilot CLI |
|---|---|
| Grid preview, status and Inbox | Yes |
| [Chat](../app/chat-terminal.md) and Terminal | Yes |
| Blocked notifications | Yes, without Approve or Deny |
| Answering prompts | Terminal buttons or typing in the composer |
| Key shortcuts just for Copilot CLI | Yes |
| New task | Yes |

## Set up

Install Copilot CLI on the machine as `copilot`, and keep it in the `[tasks] agents` list of `collied.toml`, where it is by default. herdr starts it and reads its status from the screen.

## Chat

Chat shows the conversation that Copilot shows in the pane, and follows it when you start another one with `/new`. A subagent's steps appear under the task that started it. Copilot names a session after your first prompt. Until it does, or when two conversations in the pane share a name, Chat says it cannot tell which conversation the pane shows rather than guessing, and Terminal still works.

## Terminal

Terminal hides Copilot's scrollbar column. With line wrapping on, Copilot's prose and your sent prompts wrap to the phone's width.

## Startup prompts

When Copilot asks whether to trust the folder, the new task stops with a message. Answer the prompt in Terminal, with the key strip or by typing in the composer.
