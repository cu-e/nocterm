# 0083. The status bar is zoned and drawn from a resolved layout

Status: accepted

## Context

The status bar was a fixed sequence of toggles and views that each feature added by
call, and the swap of sides moved the sidebar and the side panel as two special cases.
Adding a widget meant choosing its place in code, and nothing could say where a panel
toggle belongs without a view to ask. Contributions now carry placement hints
([ADR 0082](0082-system-plugins-and-core-contributions.md)), so the bar can be derived
from data.

## Decision

The bar has three zones, left, centre and right. A **layout** arranges the contributions
of the catalog in them: widgets, separators and panel groups, a group holding panel
toggles and separators. `nocterm-window-layout` resolves a layout against the catalog
into a `ResolvedLayout`, a pure function with no I/O.

- A built-in layout is never stored. `Default` is the empty preset: every contribution
  whose placement asks to be shown is placed where its hints say, in `(zone, order)`
  order. A panel joins the group it names, or else the primary group for a navigation
  panel and the other group for an assistant, and a group joins the zone of its first
  panel. `Swapped` is the same arrangement with every group on the other side of the
  content; the bar itself is unchanged.
- A group opens on the side its zone implies (left and centre on the left, right on the
  right) unless the layout names a side. The swapped flag of the existing
  `workspace.swapped` setting selects between the two built-ins until layouts are
  stored.
- A contribution is placed once; ids the catalog does not know and ids of the wrong
  kind are not placed but are never dropped from a preset. Separators never lead,
  trail or double in a zone or a group. Exactly one group is primary and is in the bar
  even without members; any other group needs a panel to stay.
- A **pinned** contribution (the notice bar) cannot leave the bar: if the layout does
  not place it, it goes to the end of the right zone.
- A panel that the layout leaves out stays registered. While it asks for the user
  (attention) the shell draws its toggle at the inner end of the right zone, so a
  permission request is never hidden by a layout.
- A toggle shows its contribution's title and icon; while the panel asks for the user
  it shows the shield and says so, whether or not the panel is open.
- Widget views are created lazily: the workspace keeps each widget's factory and
  creates its view the first time the layout places it.
- Hiding a group moves focus only when focus was inside it: to the visible expanded
  group if there is one, else to the active item.

The workspace draws the bar from the resolved layout. `ToggleRightPanel` and
`ToggleRightPanelMaximized` act on the assistant group, and panels are shown, hidden
and queried by contribution id.

## Alternatives

- Keep per-feature calls that add views in order: the order stays implicit and a layout
  cannot move anything.
- Store the built-ins: they would have to be migrated whenever a plugin is added.

## Consequences

- A widget or panel appears in the bar by declaring a placement; the default bar looks
  as before.
- The pre-plugin calls for adding panels and status views are removed.
- Built-in layouts are resolved against the catalog when the window is composed, not stored.
