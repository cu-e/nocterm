# 0156. Agents work with the files of terminals that run no programs, through tools offered only then

Status: accepted. Extends [0128](0128-agents-transfer-files-through-the-shared-transfer-runtime.md)
(terminal filesystems) and [0155](0155-a-plugin-terminal-tab-can-serve-files-to-the-file-browser-and-ai-chats.md)
(plugin terminals with files).

## Context

An agent changes a server's files by running programs there (`exec_command`, `run_command`),
and copies them with `transfer_files`. A plugin's console tab with files
([0155](0155-a-plugin-terminal-tab-can-serve-files-to-the-file-browser-and-ai-chats.md)) serves a
filesystem but runs no programs, so an agent attached to it could type into the console and
copy whole files away and back, but not read or change a configuration file in place. The
`nocterm` MCP catalog was static: a tool for such terminals would be listed to every agent,
including the many with no such terminal, and spend their context.

## Decision

1. **Terminals that take file tools.** A terminal takes the file tools when its
   `TerminalFiles` gives a remote filesystem and its `TerminalExecution` is unsupported: the
   agent reaches its files in no other way. The chat decides this per attached terminal, marks
   it `file_tools: true` in `list_terminals` and the prompt context, and refuses the file tools
   on any other terminal, so they never reach this computer or a shell host.
2. **The tools.** `list_files`, `read_file`, `write_file`, `edit_file`, `make_directory`,
   `move_path` and `delete_path` (`nocterm-ai::tools::files`). Paths are absolute without
   empty, `.` or `..` parts. Files are UTF-8 text read whole up to 1 MiB and answered 64 KiB of
   lines at a time; `write_file` takes 512 KiB and refuses an existing file unless `overwrite`;
   `edit_file` replaces exact text that occurs once (or every occurrence on request), so an agent
   sends only what changes and the change happens on the file's host. The work runs over the
   captured `RemoteFs` and is a pure rule of `nocterm-ai`; the chat only resolves the terminal,
   applies approval and answers.
3. **Offered only while needed.** Catalog rows carry a `ToolGroup`; a `ToolOffer` says which
   groups a chat offers beyond the terminal tools. The chat computes it from its attachments
   when its session is leased, when an attachment changes and with each prompt, and gives it to
   its `BridgeRegistration`. The bridge lists and admits only offered tools, declares
   `listChanged`, and on a change writes `notifications/tools/list_changed` to the chat's
   connected agents from a thread of its own, through the same per-connection writer as its
   replies, so a busy agent never blocks the chat and a notice never splits a reply.
4. **Approval.** Reads follow the read policy. Changes follow the write policy under a new
   `ToolCapability::Files`, granted per terminal by **Allow file changes on this terminal**;
   the card shows the path and the text written or replaced.
5. **Secrets.** With redaction on, `read_file` hides recognised secrets as `[REDACTED]` and
   says so; `write_file` then refuses content holding the marker, and `edit_file` matches the
   file's real text, so a hidden secret is never replaced by its marker.

## Alternatives rejected

- **Tools declared by plugins.** Every plugin would define, parse and bound its own file
  tools, and their descriptions would be the plugin's words in the agent's context. The
  filesystem a plugin already serves gives the same reach with the host's rules.
- **File tools for every terminal with a filesystem.** Shell hosts already have programs for
  this; listing the tools for every SSH chat costs every agent context for little.
- **Only `transfer_files`.** Copying a file to this computer to change one line needs a local
  terminal, two transfers and the user's attention for each.
- **A static catalog with the tools always listed.** Agents without such a terminal would carry
  seven unusable tools.

## Consequences

- An agent that does not handle `notifications/tools/list_changed` sees the file tools only the
  next time it lists its tools, such as in a new chat; calls to tools not offered are refused
  with the reason.
- The tool catalog fixtures grow with the seven rows; the default `tools/list` is unchanged.
- The file tools stop at 1 MiB and at text; `transfer_files` remains for larger or binary files.
