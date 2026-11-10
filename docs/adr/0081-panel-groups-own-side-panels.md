# 0081. Panel groups own the window's side panels

Status: accepted

## Context

The workspace held its sidebar and its independent side panel in two owners with
parallel rules: each tracked visibility, the selected panel, attention and focus
handoff, and the body laid out exactly two named columns. A third panel position
would have meant a third owner and another column variant, and every rule would
have been written once per owner.

## Decision

`PanelGroups`, private to the workspace shell, owns every side panel. A group
(`GroupKey`) has a side, a width class and a list of members; its pure state
records whether it is shown, which member is selected and whether it is expanded
over the content. The invariant is that an expanded group is shown and has a
selected member. A `HostedPanel` erases the panel type behind a handle, caches
its title, icon and badge, keeps its attention flag and owns the observer
subscription. Slots are addressed by a permanent `PanelKey`, so closing or hiding
a panel never moves another one.

Two fixed groups exist today. `Navigation` is the primary group: it holds the
sidebar panels, starts shown on the left and its toggle hands focus to the
expanded group's panel or the active tab. `Assistant` holds the side panel on the
right; it opens only while its panel is available, focuses the panel on opening
and returns focus to the content when it closes with focus inside.

Operations return a named `PanelTransition` (the group whose visibility changed
and the focus change to apply) in the pattern of ADR 0044. The workspace applies
it after the state commits, and emits `RightPanelVisibilityChanged` only when the
changed group is `Assistant`.

The body derives an `Arrangement` from the shown groups: one column per shown
group, the content column unless a group is expanded, and each group on its
configured side, mirrored when the sides are swapped. Within a side the groups run
from the window edge inward. Widths are remembered per width class (the existing
`workspace.sidebar` and `workspace.right_panel` keys), one resize state exists per
arrangement, and measurements taken while a group is expanded never overwrite the
remembered docked widths. A column draws either the workspace's own title above the
panel or leaves the whole column to a panel that draws its own header.

## Alternatives

- Add a third owner for each new position: duplicates the rules and the column
  variants for every panel the shell gains.
- Keep two owners and share their rules through a trait: the owners would still
  decide layout and focus separately.

## Consequences

- A new group is a new `GroupKey`, a side, a width class and a column; no owner,
  event or arrangement variant is added.
- `Workspace` loses the sidebar and side-panel owners (eight fields become seven).
- The behaviour of the existing shell is unchanged; characterization tests pin
  the focus transitions and the default geometry.
- Supersedes in part ADR 0006 (the side-panel slot state machine) and ADR 0026
  (the `Sidebar` and `RightPanelSlot` owners).
