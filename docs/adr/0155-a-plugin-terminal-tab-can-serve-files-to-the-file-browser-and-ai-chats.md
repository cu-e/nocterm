# 0155. A plugin's terminal tab can serve files to the file browser and AI chats

Status: accepted. Extends [0153](0153-toolbars-and-user-plugin-requests-timers-and-terminals.md)
(terminal tabs, requests).

## Context

A plugin that reaches a server of the user's, such as a game server panel, could show a console
as a terminal tab, but its files only in a list of its own. The application's file browser, and
the file tools of an AI chat attached to the tab, knew nothing of them: the agent could type
into the console but not read or change a configuration file. Network answers reached the
plugin as text, so a binary file lost bytes on the way.

## Decision

1. **A tab with files.** `terminals.open` takes `files`; such a tab needs the further
   capability `terminals.files`, which the host checks when it applies the effect. The
   application gives the tab's session a file system (`PluginFs`, a `RemoteFs`) in place of
   none, so the file browser shows it while the tab is active and an attached chat's file tools
   reach it through `TerminalFiles`, exactly as for an SFTP session.
2. **Requests and answers.** Each request of that file system becomes a `file-request` event
   (`list`, `stat`, `read`, `write`, `create-dir`, `remove`, `rename`) under a number; the
   plugin answers with the `terminals.files` effect under the same number (`listed`, `found`,
   `content`, `done`, `failed`). At most eight requests of a tab wait at once and each waits at
   most 30 seconds; a closed tab's requests fail.
3. **Checked both ways.** The host asks only about absolute paths without `.` or `..` steps,
   NUL or doubled slashes, and accepts only plain entry names (no `/`, control, invisible or
   direction characters), at most 1000 entries a listing and 160 KiB a file. Nothing the plugin
   answers is a file of this computer; downloading one to this computer is the user's transfer,
   as from any server.
4. **Bytes.** File contents travel as base64. `net.request` takes `bytes`: the body is sent as
   raw bytes and the answer comes back as base64 of what the server sent, at most 160 KiB, and
   is refused rather than cut above that. Base64 is in the contract, so a plugin passes it on
   without decoding.

## Alternatives rejected

- **A file browser drawn by the plugin.** It cannot be the agent's, and repeats the
  application's own.
- **A manifest table of file endpoints.** The host would have to know each server's API; the
  plugin already speaks it.
- **Text contents.** A binary file would change when it is written back.

## Consequences

- The surface, the ABI golden samples and the reference grow with the capability, its event and
  effect, `files` and `bytes`.
- Large files (above 160 KiB) are refused through a plugin; a server's own download remains for
  them.
