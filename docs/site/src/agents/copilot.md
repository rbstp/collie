# Run Copilot CLI from your phone

GitHub Copilot CLI agents are supported through Terminal.

## What works

| Feature | Copilot CLI |
|---|---|
| Grid preview, status and Inbox | Yes |
| Terminal | Yes |
| Chat | No |
| Blocked notifications | Yes, without Approve or Deny |
| Answering prompts | Terminal buttons or typing in the composer |
| Key shortcuts just for Copilot CLI | Yes |
| New task | Yes |

## Set up

Install Copilot CLI on the machine as `copilot`, and keep it in `collied.toml`'s `[tasks] agents` list (it is there by default). herdr starts it and reads its status from the screen.

## Terminal

Copilot's scrollbar column is removed, and its prose and sent prompts wrap to the phone's width with line wrapping on.

## Startup prompts

Copilot's folder trust prompt stops a new task with a message. Answer it in Terminal, with the key strip or by typing in the composer.
