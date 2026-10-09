# 0002. Item commands are GPUI actions dispatched to the command item

## Context

Menus and the title bar ran item commands through an `ItemCommand` enum,
`Item::command_enabled`/`execute` and an `item_action!` macro, duplicating
what GPUI's dispatch tree already knows: which element handles an action.

## Decision

Items handle Find, recording, session and edit commands as ordinary actions on
their own elements. The workspace forwards an action from its chrome to the
command item's focus handle, and a menu entry is enabled only when that item
would handle it (`is_action_available` on the last rendered frame). The
workspace registers no handler of its own for these actions, so availability is
never "always".

## Alternatives rejected

- A typed registry keyed by `TypeId` (`register_command::<A>(enabled, run)`):
  duplicates the dispatch tree.

## Consequences

- A new terminal command touches only the terminal crate and a menu line.
- Action types stay where they were declared so keymap names do not change.
- The forwarder is installed only on chrome containers; on the workspace root
  `command_available` is false, since the root is an ancestor of everything.
- Commands that take focus are deferred to after the current frame.
- Availability lags one frame; items notify on state changes, which suffices.
