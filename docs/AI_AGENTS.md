# AI agents

The AI panel sits to the right of the workspace and stays available when no
terminal tabs are open. Open it from the footer, Window → AI Agents, or
Ctrl+Alt+I (Cmd+Alt+I on macOS). Settings remains available from Session →
Preferences → Settings and Ctrl+, (Cmd+, on macOS). The footer no longer has a
Settings button.

Create a chat with Claude, Codex, Hermes, or a custom ACP executable. Agent
processes and the tool listener start only when needed. The panel can occupy the
window's working area; its maximize control keeps the footer and does not change
the operating system's fullscreen state. Chat history lives in memory for the
current window. Closing the app or switching AI off clears it. Model favorites
are stored separately in `agents.toml` under the state directory.

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

AI Settings controls the master switch, default agent, working directory,
terminal approvals, output filtering, and executable configuration. Its changes
are drafts until Apply. Disabling or removing an agent retires its current
connections. Changes to an agent's command, arguments or environment apply to new connections; existing
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

## Context and controls

A new chat attaches the active terminal, if one exists. The context menu can
attach several terminals, connections or a connection group. Connection and
group attachments resolve their open terminals live. Opening a connection is a
user action through the normal connection/authentication UI. Agents cannot open
SSH connections or use the authentication UI as a terminal tool.

The agent receives explicit terminal descriptors: opaque terminal id, title,
local/remote status, working directory when known, and selected connection
metadata (name, group, description, host, port, user). Descriptor text is filtered
and encoded as JSON context. It never serializes a complete profile, SessionSpec,
Auth, credential id, private-key path, proxy configuration or shell launch.
Context bytes sent by Nocterm are distinct from the agent's reported context
window tokens/cost; unknown agent usage is not estimated.

Modes, models, effort and image input depend on the agent's real ACP
capabilities/configuration. A missing capability does not mean that a default
model or token count can be invented. Model favorites are keyed by agent,
configuration option and value. Image input accepts validated PNG, JPEG, GIF and
WebP, at most 5 MiB per image, eight images and 20 MiB per prompt. Decoding is
bounded to 4096 pixels per axis, 16 million pixels and 64 MiB allocation.

## Terminal tools and approvals

The `nocterm` MCP server offers only tools for terminals attached to the chat:

| Tool | Default approval | Bound |
| --- | --- | --- |
| `list_terminals` | None | Attached terminal descriptors only |
| `read_terminal` | Allow | 2000 logical lines / 64 KiB |
| `send_input` | Ask | 16 KiB input |
| `run_command` | Ask | One command line, at most 16 KiB |

Approval displays the exact target and input. Grants belong to one terminal in
one chat; a new member of a group does not inherit an existing grant. Detaching
or closing a terminal prevents further tool calls. The application checks the
attachment and terminal state again after approval immediately before input.
Connecting, closed, and authentication states refuse input. Command execution
also rejects alternate-screen programs and known busy/dirty local prompts.

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
The composition root injects adapters through domain contracts. Features never
import ACP adapters or the terminal/connections features; Workspace supplies
neutral `TerminalAccess` and `ConnectionDirectory` contracts.

ACP carries chat messages and configuration. ACP client filesystem and new
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

**Local agents are not sandboxed. They run as your user and may read your files
with their own tools, including `~/.ssh`.** Nocterm's allowlisted context and
environment policy do not prevent that access. A strict guarantee that an agent
cannot read user secrets would require additional operating-system isolation.

Nocterm excludes its SSH/vault authentication fields from terminal context.
Authentication answers are handled outside terminal scrollback. However,
connection descriptions, host/user metadata and terminal output can contain
secrets supplied by the user or remote program. Output filtering recognizes
private-key blocks, common token prefixes and common password/token assignments;
it cannot recognize every secret. Do not attach terminals that show sensitive
output. Error/stderr display also applies filtering; it remains best effort.

A process with the same user's privileges and a stolen bridge token can use the
registered tools while the token is valid. Approval/attachment checks still
apply. Unix directory modes do not isolate against other processes running as
the same user. Windows loopback authentication does not promise Unix ACL
behavior. AI provider retention and external agent tool behavior are governed
by the selected provider/agent, not by Nocterm.

## Verification

Domain tests cover stable ACP updates, environment blocks, input limits,
metadata filtering, image decoding limits and favorites persistence. Adapter
tests use scripted ACP traffic and real local sockets/processes; feature tests
exercise model state and element ids without screenshots. Real-agent smoke
checks are opt-in and require local installations/authentication. A successful
build or fake-agent test alone is not an end-to-end result for Claude, Codex or
Hermes.

The new `[ai]` table uses strict schema validation. Older Nocterm versions that
do not know this table may reject those settings; keep a backup before opening
the same settings with an older version.
