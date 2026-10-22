# 0065. A window retains its context surfaces

Status: accepted

## Decision

Each terminal surface owns its native dock, tabs and background sessions. Hiding
it preserves running sessions and scrollback; closing it closes its own Items.
The dock remains the single owner of pane order and topology.

Source receipts identify the original window and surface. Separate selection
receipts reject stale menus and focus callbacks, including switching away and
back. Hidden completions update their source without changing visible selection.
A newly created Item rejected by a closed destination is closed normally.

Workspace folder tabs use these surfaces as described in
[ADR 0067](0067-project-tabs-with-inner-surfaces.md).
