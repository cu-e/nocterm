# 0159. Plugin views draw charts, a toolbar knows its axis, and moving a toolbar is a layout edit

Status: accepted. Extends [0154](0154-plugin-views-use-the-applications-components-and-external-plugins-share-the-plugins-page.md)
(plugin views from the application's components) and
[0158](0158-a-plugin-toolbar-keeps-to-its-own-tabs-and-the-user-places-it.md) (plugin toolbars
in their own tabs).

## Context

A plugin that shows a server's load had meters and figures but no way to show how the load
went, which the application's own monitor shows as small area charts. A plugin toolbar moved
to a side was stacked as a column, but the plugin could not tell, so its long figures widened
the rail. The text of a toolbar inside a tab took the tab's base size, larger than the status
bar's. And a toolbar moved within a layout of the user's own stayed where it was in the tabs:
the window did not count a change of the toolbars alone as an edit of the layout.

## Decision

1. **Charts.** `NodeKind::Chart { series, max, span }` is appended to the view's node kinds:
   one or two series of at most 120 values (`MAX_CHART_SERIES`, `MAX_CHART_POINTS`), drawn
   from 0 to `max` (the largest value when `None`), the width holding `span` values so a short
   history grows in from the right. The host draws it close to the monitor's graphs, a light
   fill under a line, in the theme's chart colours (the first series takes the node's tone
   unless it is the default): full width and 44 pixels high in a panel, a 56 by 16 pixel line
   of the newest 28 values in a bar. It is built of plain boxes, a slot per value, so the view
   crate needs no painting call of the window and the isolation rules stay as they are. The plugin keeps the history; the SDK builds it with `Chart`. The ABI stays 1:
   the variant is appended and the golden file gains a sample. Templates have no chart.
2. **The axis.** A toolbar's calls name its axis in the context's zone, which a toolbar had no
   use for: `Center` across the top or the bottom, `Left` or `Right` along that side. The SDK
   reads it as `cx.is_column()`, and `testing::Host::render_bar_in_column` draws a column. The
   wire is unchanged.
3. **Small text.** A strip or a rail sets the status bar's small text size, and a tab's strip
   is 28 pixels high.
4. **A toolbar move is an edit.** `transition` compares the toolbars of the selected layout
   too, so the window re-resolves, places its toolbars and tells the tabs that draw them.

## Alternatives rejected

- **Reusing the monitor's sparkline.** It is time-based, typed to the monitor's series and
  private to its feature crate; the plugin view crate depends only on the API and gpui.
- **Painting a path on a canvas.** It needs `Window::paint_path`, a receiver method the
  isolation rule I3 keeps from the plugin view crate unless listed with a reason; slots of
  plain boxes look close enough without widening it.
- **A field for the axis in the snapshot.** It would change the snapshot's bytes on the wire.

## Consequences

- A view's reply can be larger: two series of 120 values per chart node.
- The first series' colour follows the theme's first chart colour, not the text colour.
