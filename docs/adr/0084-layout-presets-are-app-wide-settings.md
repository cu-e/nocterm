# 0084. Layout presets are app-wide settings

Status: accepted

## Context

The window had two arrangements, the sidebar and the side panel on their usual sides or
swapped, and a command that flipped between them and wrote a flag to `layout.json`. The
zoned status bar ([ADR 0083](0083-zoned-status-bar.md)) can arrange any contribution, so
users need to keep several arrangements, name them and switch between them, in every
window.

## Decision

The arrangements are the `[layouts]` section of `settings.toml`, owned by
`nocterm-window-layout`. It holds `active`, the selected layout, and `presets`, the layouts
the user saved. Each preset has an id, a display name, the three zones of the bar, a
`tray` of registered contributions that stay unplaced and `open`, the panels to show when
the layout is selected.

- The built-ins `default` and `swapped` are never stored and always available by id.
  A stored preset has an id `p<n>`, with n one more than the largest stored. Sanitizing
  gives a missing, invalid, reserved or repeated id a fresh one, shortens and numbers
  names so that they differ from each other and from the built-ins, and bounds the
  count and the length of every list. When an invalid id of the selected stored preset is
  replaced, `active` follows it. At the largest number the next id is the smallest unused
  one.
- `select` picks `active` if it names a layout that exists, else `swapped` while the
  earlier `workspace.swapped` flag of `layout.json` is set, else `default`. It never
  writes: a user who swapped the sides before keeps that arrangement without a write at
  startup, and an unknown `active` stays in the document. Nothing sets the flag any more.
- `transition` compares two documents and tells a window what happened. A **switch**
  changes the selection between layouts that both documents have; the window then shows
  exactly the panels the new layout opens, and a built-in leaves the open panels as they
  are. An **edit** changes the selected layout's bar, tray or `open` list, or creates or
  removes the layout; the window rearranges itself and keeps the panels that are still
  placed open, and a panel the layout no longer places closes. A rename or a change to
  another layout is no change, and the window does nothing.
- On every change the panel groups take their members, their order and the placement of
  every registered panel from the resolved layout, so a panel the layout puts in the
  assistant group is shown by the assistant toggle and the footer, and the sidebar toggle,
  next-panel and `set_open` act on the groups as drawn. A panel the bar leaves out keeps
  the group its hints name.
- Every window observes the shared settings handle and applies each published change to
  its own panels, so a switch made in one window reaches all of them. A switch is a
  checked write that fails if the layout was removed in the meantime; a failed write
  leaves the selection as it was and the user sees a notice. A `[layouts]` table that
  does not read keeps its raw text until the section is changed, and a switch would
  replace it with the defaults; the switch is refused with a typed error instead. A window composed under a
  saved layout opens the panels the layout names as it registers them.
- The Window menu lists the layouts with the selected one checked, and the panels the
  layout places. `SwitchLayout` and `TogglePanel` carry an id, are not in the palette and
  have no binding; `NextLayout`, `PreviousLayout` and `PickLayout` are palette commands.
  `workspace::SwapSides` is removed.

The compiled-in plugins and their contributions are described by a generated
`docs/reference/plugins.md`, which a test compares with the application catalog.

## Alternatives

- Keep layouts per window: the choice would be lost with the window, and two windows could
  disagree about where the sidebar is.
- Store the built-ins: they would have to be rewritten whenever a plugin is added.
- Keep the flag and add presets beside it: two sources of truth for the selection.

## Consequences

- One selection and one list of layouts for the application, persisted and ordered like
  the other settings.
- Keymap entries for `workspace::SwapSides` are skipped and logged as an error.
- Editing layouts from the interface builds on `transition` and the same settings
  section.
