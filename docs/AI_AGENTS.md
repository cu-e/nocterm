# AI agents

The AI panel sits to the right of the workspace and stays available when no
terminal tabs are open. Open it from the footer, Window → AI Agents, or
Ctrl+Alt+B (Cmd+Alt+B on macOS). Ctrl+E (Cmd+E) moves it to the left of the
tabs and the sidebar to the right, and back; on the left, the history column
opens on the panel's left too. Settings remains available from Session →
Preferences → Settings and Ctrl+, (Cmd+, on macOS). The footer no longer has a
Settings button.

Create a chat with Claude, Codex, Hermes, or a custom ACP executable; the
built-in agents show their brand marks in the menu, the panel header and AI
Settings, custom agents a generic icon. Ctrl+N (Cmd+N on macOS) in the panel
starts a new chat with the agent the last chat was started with (remembered in
`agents.toml`; before any chat, the default agent); the new-chat menu shows the
shortcut beside that agent. Agent
processes and the tool listener start only when needed. The panel can occupy the
window's working area; its maximize control keeps the footer and does not change
the operating system's fullscreen state. Chats are saved and come back after a
restart (see [Chats and history](#chats-and-history)). Model favorites are
stored separately in `agents.toml` under the state directory.

## Agent setup

The built-in commands use explicit adapter versions:

| Id | Command | Authentication environment inherited by default |
| --- | --- | --- |
| `claude` | `npx -y @agentclientprotocol/claude-agent-acp@0.85.1` | `ANTHROPIC_API_KEY`, `ANTHROPIC_BASE_URL`, `CLAUDE_CONFIG_DIR` |
| `codex` | `npx -y @agentclientprotocol/codex-acp@2.1.1` | `OPENAI_API_KEY`, `OPENAI_BASE_URL`, `CODEX_HOME` |
| `hermes` | `hermes acp` | `HERMES_HOME` |

Install the appropriate CLI/adapter prerequisites yourself. The Claude adapter
requires Node.js 22 or later; Codex also requires a working Node.js/npm setup.
Hermes must be installed and available in PATH. First startup through `npx` may
need network access to download the pinned package. These pins describe the
configured defaults, not proof that all agents have been authenticated or
successfully tested on your machine.

Desktop applications may receive a different PATH from an interactive shell,
particularly with nvm. Set an absolute executable path in AI Settings when the
program cannot be found. Arguments are a JSON array of individual strings;
Nocterm does not run them through a shell. A blank override keeps the built-in
value; an empty arguments array removes the built-in arguments. Custom agent ids
use letters, numbers, underscore or dash and need an executable.

AI Settings (Settings → AI agents) controls the master switch, default agent,
working directory, terminal access approvals, secret filtering, isolation and the
agents themselves. Every change saves itself; text fields save when typing pauses,
on Enter and when they lose focus, and invalid text is kept with its error rather
than saved. Each agent folds open to show its name, executable, arguments and
environment; a custom agent is added with an id and an executable. Disabling or
removing an agent retires its current connections. Changes to an agent's command, arguments or environment apply to new connections; existing
chats keep their current process. A custom working directory must be absolute;
the default is a private `agent-workspace` directory in application state.

Environment overrides are plain text in settings: do not store secrets there.
Recognized credential variable names (such as API key, token and password
names) are rejected as overrides; arbitrary secret values cannot be classified.
Use inherited environment names for existing API key variables you deliberately
want an agent to receive for authentication. The process environment is cleared
and rebuilt from allowed desktop/runtime variables plus explicitly inherited
names and validated overrides. SSH agent, GPG agent, Nocterm internal variables,
and process injection variables are blocked even when explicitly requested.

## Chats and history

Each chat is saved as one JSON file in `agent-chats/` under the state directory
(private 0700 directory, 0600 files). A chat is written when you send a
message, when the agent finishes a reply, when its title, name or pin changes,
when it fails, and once more when the application quits. A chat nobody wrote in
is not saved, and an empty chat is dropped when you start or open another one.

A saved chat holds the agent id, the agent's title, your name for it, the pin,
the agent session id and its working directory, the last model, and the
transcript: your messages, the agent's replies and reasoning, and tool calls
with the input and output the agent reported for them. The terminal descriptors
sent with each prompt are not saved, but a tool call's output can contain
terminal text. Images over 512 KiB are replaced by a note, only the last 2000
entries of a chat are kept, and at most 200 chats are restored, pinned ones
first.

After a restart, saved chats appear in the history without starting their
agent. Opening one connects the agent and reopens its session with
`session/resume` (no replay) or `session/load`, whichever the agent advertises.
An agent with neither, or one that refuses, starts a new session; the status
line then says the agent does not remember the earlier messages. Saved chats
load into the first window that opens the panel, so two windows never write the
same file. Turning AI off and on again shows them only after a restart.

The clock button opens the history as a column beside the chat rather than in
its place: the panel widens by the column's width, and closing it gives the
width back. Dragging the divider resizes the column; the panel's and the
column's widths are remembered across restarts in `layout.json` in the state
directory, with the sidebar's. On the left of the window the column is on the
chat's left. Opening a chat from the history keeps the column
open. With no chat open, the history fills the panel.

The history lists pinned chats first, then the most recently changed. Its search
field matches titles, names and the text of messages, ignoring case. Each row
opens its chat anywhere it is clicked; its `…` menu offers:

| Action | Effect |
| --- | --- |
| Rename | Edit the name in place; Enter or leaving the field saves, Escape cancels. An empty name returns to the agent's title. |
| Fork | A new chat with the same messages and attachments, opened at once. The agent continues in a copy of the session (`session/fork`) when it advertises forking; otherwise the copy starts a new session that does not remember the messages. |
| Pin / Unpin | Pinned chats show a pin before the title and stay at the top. |

The trash button deletes the chat and its file.

Your messages and the agent's replies can be selected and copied with the
mouse. Hovering a message shows, under it, when it was sent (the clock time
today, the date on earlier days; chats saved before times were kept show none),
a Copy button that copies the message's text, and a Fork button. Forking from
the last message is the same as Fork in the history menu. Forking from an
earlier message copies the chat up to and including that message, and the copy
starts a new agent session: the agent cannot be made to forget what came after,
so it does not remember the copied messages either.

Attached images are shown as a row of thumbnails above the message field, each
with its own remove button, and in a row inside the sent message. Tool output
is shown as the text the tool returned, wrapping long lines, and as JSON only
for other content.

Stop cancels the current turn (`session/cancel`). Updates the agent sends after
answering the cancellation are dropped, and the next prompt continues in the
same session. Restart chat appears only when the session itself is gone: the
agent exited, failed, or did not answer the cancellation within 10 seconds. It
starts a new agent process and keeps the chat, its name and its history file.

## Context and controls

A new chat attaches the active terminal, if one exists. The `+` menu of the
composer has two parts:

- **Active sessions**: every open terminal — your tabs, the bottom shell, and
  sessions agents opened in the background (with a **Show** button that moves
  one into a tab). Attaching one lets the agent use exactly that terminal.
- **Saved servers**, laid out as in the sidebar: connections outside folders
  first, then each folder with its connections indented under it, all by name.
  Clicking a server attaches it; clicking a folder attaches every server in it.
  A green dot marks servers with an open session. **Open** opens the server in
  a new tab through the normal connection and sign-in flow.

An attached server (directly or through its folder) gives the agent every open
session of that server, whether you opened it or an agent did. A server without
a session is listed to the agent as an *offline server*; the agent connects to
it with the `open_terminal` tool, in the background, without opening a tab or
taking focus. If the connection needs you, the agent waits up to three
minutes; only a new host key moves the session into a tab so you can answer. Background sessions a chat opened end when the chat
closes or no longer attaches the server. Credentials remembered in the vault and
key or agent authentication let background sessions connect without a prompt.
A password, passphrase or one-time code is asked in the chat: the session stays
in the background, you type the answer into the card and the agent continues.
When the saved credential is in a locked vault, the card also offers **Unlock
vault**: it opens the unlock dialog (master password or fingerprint) and the
saved secret then answers the sign-in.

The agent receives explicit terminal descriptors: opaque terminal id, title,
local/remote status, working directory when known, and selected connection
metadata (name, group, description, host, port, user). Descriptor text is filtered
and encoded as JSON context. It never serializes a complete profile, SessionSpec,
Auth, credential id, private-key path, proxy configuration or shell launch.
Context bytes sent by Nocterm are distinct from the agent's reported context
window tokens/cost.

Modes, models, effort and image input depend on the agent's real ACP
capabilities/configuration. A missing capability does not mean that a default
model or token count can be invented. Model favorites are keyed by agent,
configuration option and value. Image input accepts validated PNG, JPEG, GIF and
WebP, at most 5 MiB per image, eight images and 20 MiB per prompt. Decoding is
bounded to 4096 pixels per axis, 16 million pixels and 64 MiB allocation.

## Usage and plan limits

The ring left of the send button shows how full the agent's context window is.
Clicking it opens the Context Usage card over the chat, above the composer;
the ring or the card's close button hides it. The card shows what the agent
reports and nothing it does not:

| Section | Source | Agents |
| --- | --- | --- |
| Percent full, used / window tokens, session cost | ACP `usage_update` | All that send it |
| Split of the used tokens | Estimated by Nocterm | Shown with the total |
| Session tokens: input, cache read/write, output, reasoning, total | `usage` of the prompt response (unstable ACP) | Claude, Codex, Hermes |
| Plan limits: 5-hour, weekly, per-model weekly, extra usage | See below | Claude, Codex |

ACP reports only the total in the context window. The split into "System
prompt, tools & rules", "Messages", "Reasoning" and "Tool calls" is estimated
from the transcript at about four characters per token. The remainder of the
reported total is attributed to the agent's own instructions and tools. When
the agent has compacted its context, the transcript's parts are scaled down to
fit the total. The card says the split is an estimate.

Plan limits are not part of ACP:

- **Claude.** The Claude adapter attaches Claude Code's rate-limit event to
  `usage_update` as `_meta["_claude/rateLimit"]` (window, utilization, reset
  time, status). Each report updates its window. Claude sends them only after
  replies, and not every window in every reply.
- **Codex.** The Codex adapter does not forward limits. Codex itself writes them
  into its session logs, so Nocterm reads the newest
  `$CODEX_HOME/sessions/YYYY/MM/DD/rollout-*.jsonl` (default `~/.codex`). It
  reads only the last 256 KiB of the newest log, read-only. It reads after each
  Codex reply and when the card opens. Codex logs record the limits of the
  account, not of one session.
- **Hermes and custom agents** report no limits; the card shows none.

Limits are kept per agent for the running application, so a new chat with the
same agent shows the last known values. Each limit shows the share used, a bar
(warning from 70%, danger from 90% or when reached) and the time until it resets.

## Terminal tools and approvals

The `nocterm` MCP server offers only tools for terminals and servers attached to
the chat:

| Tool | Approval setting | Bound |
| --- | --- | --- |
| `list_terminals` | None | Attached terminal descriptors and offline servers only |
| `read_terminal` | Ask before reading a terminal (off by default) | 2000 logical lines / 64 KiB |
| `send_input` | Ask before typing or running commands (on by default) | 16 KiB input |
| `run_command` | Ask before typing or running commands | One command line, at most 16 KiB |
| `open_terminal` | Ask before typing or running commands | One attached offline server |

The settings apply at once, including to running chats. An approval card shows
the exact target and input with **Deny**, **Allow once** and **Allow for this
terminal** (or server). A grant lasts for the chat's session: it survives later
turns, and is dropped when the chat restarts or fails or when an attachment is
removed. A new member of a group does not inherit an existing grant. Detaching
or closing a terminal prevents further tool calls. The application checks the
attachment and terminal state again after approval immediately before input.
Connecting, closed, and authentication states refuse input. Command execution
also rejects alternate-screen programs and known busy/dirty local prompts.

Agents also have their own permission prompts (for example before running a tool
of their own); those appear as separate cards with the agent's options.

The terminal read cursor is inclusive: a resumed read may replace the previous
last logical line after more text is appended. It is not an exact byte-stream
cursor. Alternate screens expose the visible page only. A local OSC 133 prompt
can confirm command completion; an SSH output pause only reports a pause and
cannot prove that a command completed. Cancellation/timeout must not be confused
with successful execution.

## Architecture and protocol

`nocterm-ai` is runtime-neutral domain logic: registry, environment policy,
contracts, ACP update reducer, descriptors, MCP parser, approvals, filtering,
favorites and image validation. `nocterm-acp` adapts subprocess stdio and local
bridge transport. `nocterm-agent` owns GPUI lifecycle, chat state/UI and routing.
Saved chats (`nocterm_ai::history`) and usage parsing — Claude limit metadata,
Codex logs and the transcript estimate (`nocterm_ai::usage`) — are domain code
with unit tests. The panel is split into modules by part: transcript, menus, attach menu, approvals,
composer, history and usage card (`panel/*.rs`). The Codex home directory is injected through
`AgentServices::codex_home`, so tests never read the user's logs.
The composition root injects adapters through domain contracts. Features never
import ACP adapters or the terminal/connections features; Workspace supplies
neutral `TerminalAccess` and `ConnectionDirectory` contracts.

ACP carries chat messages and configuration. Nocterm enables the schema's
unstable `session/fork` and end-of-turn token usage. It sends `session/resume`,
`session/load` and `session/fork` only when the agent's capabilities advertise
them. ACP client filesystem and new
terminal capabilities are not advertised in this version. Existing terminal
tools use stdio MCP: session creation supplies the current Nocterm executable
with the `agent-bridge` subcommand. The subcommand runs before GUI initialization,
reads endpoint/token from its environment, and relays stdin/stdout to the
application's authenticated listener. Only protocol data goes to stdout.

The listener starts lazily. Unix uses a unique socket in the path returned by
`Paths::ensure_runtime_dir()` (private 0700 directory, socket 0600); Windows uses
loopback TCP. A random 256-bit token binds each registration to its chat and is
revoked on release, chat close or AI off. Authentication has a bounded hello
and timeout. Limits include 16 MiB ACP lines, 1 MiB MCP lines, a 256-event
foreground queue, four connections per token and 64 total bridge connections.
MCP supports initialization, ping, tools listing and tool calls; unsupported
methods fail explicitly.

Switching AI off cancels prompts and pending permissions/tools, closes chats,
revokes registrations, stops the listener, terminates agent process groups, and
hides the button/panel in every workspace. Turning it on does not start anything.
Application exit uses the same teardown. Process tree termination on Windows is
best effort and does not provide Unix process-group guarantees.

## Privacy boundary

Saved chats are plain JSON readable by your user account. They contain what the
chat showed, including tool output that may quote terminal text. Delete a chat
from the history to remove its file. Reading Codex logs for limits stays on the
machine; nothing about usage is sent anywhere.

**Without isolation, local agents run as your user and may read and change any of
your files with their own tools, including `~/.ssh`.** Nocterm's allowlisted
context and environment policy do not prevent that access.

**Isolate agents** (Linux, needs bubblewrap) runs each agent process in a
bubblewrap sandbox, following the "workspace write" model of Codex and Claude
Code's sandbox:

- the file system is readable but not writable, except the working directory and
  the agent's own state and caches (`~/.claude`, `~/.claude.json`, `~/.codex`,
  `~/.hermes`, `~/.gemini`, `~/.npm`, `~/.cache`, `~/.local/state`);
- SSH and GPG keys, cloud and container credentials (`~/.aws`, `~/.azure`,
  `~/.kube`, `~/.docker`, gcloud, `gh`), password stores and keyrings, browser
  profiles, `~/.netrc`, `~/.git-credentials` and similar files, and nocterm's own
  configuration and state (including the vault and saved chats) are replaced by
  empty ones;
- `/tmp` is private; the terminal tools' socket directory stays reachable;
- the process tree gets its own PID namespace and session, so it ends with
  nocterm and cannot type into the terminal nocterm was started from.

The network stays available, because agents talk to their model provider.
Turning isolation on or off restarts running agents. When bubblewrap is missing,
an isolated agent does not start, rather than running unisolated. The terminal
tools are not affected: they act through nocterm, with the approvals above.

Nocterm excludes its SSH/vault authentication fields from terminal context.
Authentication answers are handled outside terminal scrollback. However,
connection descriptions, host/user metadata and terminal output can contain
secrets supplied by the user or remote program. Output filtering recognizes
private-key blocks, tokens of known shapes (OpenAI/Anthropic, GitHub, GitLab,
Slack, Stripe, AWS, Google, npm, Hugging Face, JWT), passwords in URLs,
`Authorization` headers and assignments whose name says they hold a secret; it
cannot recognize every secret. Do not attach terminals that show sensitive
output. Error/stderr display also applies filtering; it remains best effort.

A process with the same user's privileges and a stolen bridge token can use the
registered tools while the token is valid. Approval/attachment checks still
apply. Unix directory modes do not isolate against other processes running as
the same user. Windows loopback authentication does not promise Unix ACL
behavior. AI provider retention and external agent tool behavior are governed
by the selected provider/agent, not by Nocterm.

## Verification

Domain tests cover stable ACP updates, environment blocks, input limits,
metadata filtering, image decoding limits, favorites persistence, saved chat
files (round trip, permissions, path safety, bounds, pins and names), Claude and
Codex limit parsing and the context estimate. Panel tests cover saving,
restoring and resuming chats, stopping and continuing a session, dropped empty
chats, in-place restart, history search, pin, rename and fork, the usage card,
the sections and folder tree of the attach menu, opening a server from it, and
agents connecting to offline servers in the background. Domain tests cover the
bubblewrap policy and secret filtering; an adapter test runs an agent under
bubblewrap when it is installed. Adapter
tests use scripted ACP traffic and real local sockets/processes; feature tests
exercise model state and element ids without screenshots; the layout of the
history, the usage card and the attach menu was not checked by eye. Real-agent smoke
checks are opt-in and require local installations/authentication. A successful
build or fake-agent test alone is not an end-to-end result for Claude, Codex or
Hermes.

The new `[ai]` table uses strict schema validation. Older Nocterm versions that
do not know this table may reject those settings; keep a backup before opening
the same settings with an older version.
