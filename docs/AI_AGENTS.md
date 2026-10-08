# AI agents

The AI panel sits to the right of the workspace and stays available when no
terminal tabs are open. Open it from the footer, Window → AI Agents, or
Ctrl+Alt+B (Cmd+Alt+B on macOS). Ctrl+E (Cmd+E) moves it to the left of the
tabs and the sidebar to the right, and back; on the left, the history column
opens on the panel's left too. Settings remains available from Session →
Preferences → Settings and Ctrl+, (Cmd+, on macOS). The footer no longer has a
Settings button.

Create a chat with Claude, Codex, Hermes, or a custom ACP executable; the
built-in agents show their brand marks in the menu, the panel header, chat
history and AI Settings; custom agents show a generic icon. A working chat
shows a spinner in history; its pin remains a separate indicator. Ctrl+N (Cmd+N on macOS) in the panel
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
working directory, agent permissions, terminal access approvals, secret filtering, isolation and the
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
agent. Opening history or creating an empty chat does not start an agent or MCP.
Sending a message saves its queue entry before starting the agent and reopens its session with
`session/resume` (no replay) or `session/load`, whichever the agent advertises.
An agent without restoration, or whose saved session no longer exists, starts
a new session. A bounded copy of the saved user/assistant conversation accompanies
the next prompt, and the status explains the fallback. Temporary restoration
errors are retried once and retain the session descriptor; authentication keeps
it for retry after sign-in. Attached connections and groups survive restarts;
a remote terminal attachment restores through its saved connection. Local
shells cannot be reconnected from an earlier process. Saved chats
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
| Fork | A new chat with the same messages and attachments, opened at once. The agent continues in a copy of the session (`session/fork`) when it advertises forking; otherwise the copy starts a new session with bounded saved conversation context. |
| Pin / Unpin | Pinned chats show a pin before the title and stay at the top. |

The trash button deletes the chat and its file.

Your messages and the agent's replies can be selected and copied with the
mouse. Hovering a message shows, under it, when it was sent (the clock time
today, the date on earlier days; chats saved before times were kept show none),
a Copy button that copies the message's text, and a Fork button. Forking from
the last message is the same as Fork in the history menu. Forking from an
earlier message copies the chat up to and including that message, and the copy
starts a new agent session: the agent cannot be made to forget what came after,
with bounded saved conversation context accompanying its next prompt.

Attached images are shown as a row of thumbnails above the message field, each
with its own remove button, and in a row inside the sent message. Tool output
is shown as the text the tool returned, wrapping long lines, and as JSON only
for other content.

Typing `/` offers the commands this ACP agent advertises, including descriptions
and input hints. Up/Down select; Tab inserts the selected command, and further
Tab/Shift+Tab presses cycle the original matches without leaving the composer.
Enter or a click confirms the selection with the caret at the end. Escape closes
the suggestions. Rows show a short single-line summary and follow the keyboard selection as
you move through the list. Hovering a row shows its full description and arguments.
A recognized command followed by a space becomes a highlighted token inside the
message field; hovering that token opens the same compact card. There is no
automatic description overlay. Clicking the token selects it for replacement;
Backspace/Delete remove it as a unit, and undo restores edits. Arguments remain
ordinary text, and the sent prompt retains the original `/command args` text.
For model selection, the composer dropdown browses and searches the models the
agent advertises; its tooltip reports their count. `/model list` is not invented
or intercepted: each agent decides what its command arguments mean. An advertised `/command` and any arguments are sent as the first prompt block,
without terminal instructions or history prepended. Unknown slash names and paths
such as `/var/log/nginx/error.log` remain ordinary messages with their terminal
context and saved history. Unadorned administrative
commands remain a single text block; any saved-conversation fallback waits for
the next ordinary question. This pending context is saved across further restarts
and successful native resumes, then cleared on disk after a completed ordinary turn.

You can send another message while the agent works. It joins this chat's FIFO
queue with its text, images and attachment snapshot. The strip peeking above the
composer shows the count; clicking it unfolds the complete queue. Its height
fits the laid-out text, images and attachments; large queues scroll within a
maximum height. The arrow sends
a selected message next: it cancels the current response and waits for cancellation
to finish before sending. Other messages keep their order. The pencil edits a
queued message in place and restores your unsent composer draft after saving or
cancelling. Send-now controls are disabled while editing. Queued images are saved
in full; a message exceeding the saved chat's 32 MiB limit is rejected with its
draft intact. Restored queues wait for an explicit send-now action. Stopping,
authentication failures and session failures retain and pause the remaining queue.
A disk-write failure also pauses the queue and reports the problem. Finishing or
cancelling an edit preserves any stop or error that paused the queue during editing.

Streamed replies reveal gradually and fade in, with reduced-motion preferences
showing new text immediately. The transcript follows the latest response until
you scroll up; **Jump to latest** resumes following. Reasoning and tool details
use the secondary text color.

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
| `run_command` | Live-input approval | One command line, at most 16 KiB; known empty shell prompt required |
| `exec_command` | Separate structured-execution approval | 16 KiB program/args, 64 KiB stdin, 5 minute absolute timeout |
| `read_command` | Reading approval | Full 64 KiB stdout snapshot plus 4 KiB stderr tail |
| `cancel_command` | Structured-execution approval | One command owned by this chat |
| `open_terminal` | Ask before typing or running commands | One attached offline server |

The settings apply at once, including to running chats. An approval card shows
the exact target and input with **Deny**, **Allow once** and **Allow for this
terminal** (or server). A grant lasts for the chat's session: it survives later
turns, and is dropped when the chat restarts or fails or when an attachment is
removed. A new member of a group does not inherit an existing grant. Detaching
or closing a terminal prevents further tool calls. The application checks the
attachment and terminal state again after approval immediately before input.
Connecting, closed, and authentication states refuse input. Live `run_command`
also rejects alternate-screen programs and unknown, busy or dirty prompts,
including SSH shells without OSC 133 integration. Grants are separate for reading,
live input, structured execution and opening connections. Explicit `send_input`
into an unknown, busy or dirty shell always needs a one-time approval, even when
normal write approvals are disabled; it never consumes a remembered safe-input grant.

`exec_command` starts a fresh process through the exact authenticated connection
of the attached terminal; local terminals explicitly use the local executor.
There is no remote-to-local fallback and no inherited live-shell cwd, environment,
aliases or functions. Pass program and arguments separately; local execution
passes arguments directly, while SSH renders a POSIX command line. Arbitrary
arguments require a POSIX-compatible remote command shell; exact argument handling
is not guaranteed on Windows SSH. Use an explicit shell when shell syntax is
needed. Short commands return their actual exit status.
Long commands return a chat-owned `command_id` for `read_command` and
`cancel_command`. Cancelling a read waiter does not cancel the command. Each chat
allows four active commands and retains sixteen records; oldest completed records
are evicted. stdout is continuously drained beyond the retained 64 KiB and marked
truncated, so a noisy command can still finish and report its real status. Startup
and execution share an absolute deadline.

Stop, chat closure/restart, AI disable, detachment and session replacement cancel
owned structured executions. SSH cancellation sends TERM and closes that command's
channel; servers may ignore signals and remote descendants may survive.

Live `run_command` serializes ownership across chats only while observing its
result. Stop requests Ctrl-C only during an active observation that still owns the
same session. Accepted human input or reconnect relinquishes ownership, so a later
Stop cannot interrupt a new user command. Every observation exit releases the
lease. Observation reports completion only on a confirmed shell prompt; output
idle or observation timeout leave completion unknown, never invent an exit status,
and do not stop the shell. A later Stop no longer controls that program. Use
`exec_command` when execution needs a durable handle, cancellation and a deadline.

Agents also have their own permission prompts (for example before running a tool
of their own or accessing files). **Agent permissions → Ask before an agent uses
its own tools** controls those requests independently of terminal read and write
approvals. Its setting is `ai.approval.agent_permissions`, with `ask` as the default.
Turning it off (`allow`) selects the provider's ACP `allow_once` choice for each
request, including requests already waiting. It never automatically selects
`allow_always`: switching back to Ask makes subsequent requests wait again.
If the provider supplies no valid one-time choice, the request remains as a card
with an explanation and the provider's options for manual review. Stop, disabled
AI and requests from an obsolete or different session cancel instead of approving.

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

The conversation document, live ACP session and process container have separate
owners. A chat can stay open after its agent stops. Each live session has its own
connection, so a hung or unsupported `session/close` cannot terminate another
chat's work. Closing awaits the ACP response (up to five seconds), acknowledges a
FIFO fence after earlier foreground events, then waits for process cleanup. A
closing session still occupies its admission slot. Failed physical cleanup keeps
that slot reserved and reports the error. Late startup results whose UI owner
has disappeared are closed instead of being abandoned.

`[ai.sessions]` controls the policy shared by every window: `max_live = 4` counts
starting, live and closing sessions; `max_idle = 2` retains warm sessions;
`idle_timeout_secs = 90` releases idle sessions. Requests wait in their saved FIFO
queues when the limit is reached, and the oldest idle session yields first.
Generating, authentication, permissions, bridge calls, configuration requests and
active command jobs retain a session. A paused queue, composer edit, pin or draft
does not. Idle release leaves background terminals and shell command ownership
intact. **Release agent resources** in chat actions pauses the queue and preserves
the document; sending another message activates it again.

On Linux, agents require systemd 254 or later, `systemd-run` and a reachable systemd user manager. Each
connection uses a transient user service with `KillMode=control-group`, a bounded
stop timeout and limits for the complete process tree. `[ai.resources]` defaults
to `memory_high_mb = 2048`, `memory_max_mb = 4096`, `memory_swap_max_mb = 1024` and
`tasks_max = 512`. Exceeding a hard limit can terminate the agent. A missing user
manager prevents launch and reports an error. The internal `agent-host` helper
watches a pidfd for the owning Nocterm process, verifies its `/proc` start time
and exits when that owner dies; systemd then cleans up the cgroup, including
children that created new process groups. Child environment variables are
rebuilt from the allowlist rather than inherited from the systemd manager.

Switching AI off cancels prompts and pending permissions/tools, closes chats,
revokes registrations, stops the listener and hides the panel in every workspace.
Turning it on starts nothing. On application exit a synchronous serialized flush waits for any active atomic
write, blocks later stale writes, and saves final document snapshots before
the windows are destroyed, including dormant chats. A failed save retains its snapshot and queued messages in memory and
shows an error independently of ACP ownership. Other Unix systems retain process
group teardown; Windows process tree termination is best effort.

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
- Selecting home as the workspace keeps credential descendants masked. A workspace
  inside a credential store, including through a symlink, is refused. The dedicated
  agent workspace inside nocterm private state remains reachable.
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

Unavailable local terminal references stay visible after reopening, including beside
working remote attachments. They grant no terminal access and can be removed explicitly.
Editing queued attachments changes only that future message; the active turn and
saved chat defaults keep their original attachment scope.
