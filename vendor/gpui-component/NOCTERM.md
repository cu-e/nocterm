# Nocterm's GPUI Component menu patch

This vendored fork, including Nocterm's local modifications, is distributed
under Apache-2.0, as declared in its Cargo manifest. The application's
PolyForm Perimeter license does not apply to this copy. Upstream copyright
notices and license terms remain in force.

Origin: the crates.io `gpui-component` 0.7.0 source distribution,
[upstream GPUI Kit repository](https://github.com/longbridge/gpui-kit),
commit `0c830f4d257e69fdd17200650533ab4ca9a40cc0`, path `crates/component`.
The upstream source, copyright notices and Apache-2.0 license are retained.
This is an internal Cargo patch; the crate is excluded from Nocterm's workspace.

`src/menu/app_menu_bar.rs` changes menu behavior. `AppMenuBar::set_menus`
updates a closed menu bar's snapshot in place, retaining existing menu entities,
button identities and keyboard focus. `is_open` lets the window preserve an open
popup's snapshot. Existing global `reload` behavior stays unchanged. Nocterm
supplies per-window snapshots without replacing the process-wide menu source or
forking popup rendering, keyboard navigation or action dispatch.

Without this change, refreshing action availability before an opening event
recreated the triggers, preventing mouse or keyboard opening after state changes.
Native event regressions live in `crates/workspace/src/tests.rs`; the patched
module also tests entity retention and preservation of an open snapshot.

The published crate omits five inputs referenced by its upstream unit tests.
`tests/upstream` contains the exact icons, Markdown, theme and base-dock source
from the same pinned upstream commit; test-only include paths point there. No
upstream assertions are removed. The corresponding modified include paths are in
`src/icon.rs`, `src/button/button_icon.rs`, `src/dock/mod.rs`,
`src/text/compat.rs` and `src/theme/schema.rs`. Their original paths are
`crates/assets/assets/icons/{arrow-up,search}.svg`, `examples/fixtures/test.md`,
`themes/aurora.json` and `crates/base/src/dock/mod.rs`.

When updating GPUI Kit, compare the patched module with upstream. Preserve toolkit
action-context restoration, native input dispatch and trigger identity. Re-run
workspace menu tests and the component's menu-bar regression. Remove this patch
when upstream exposes equivalent safe per-window updates.

`src/button/button.rs` also accepts an optional application-global
`ButtonMetrics`. Nocterm projects its design tokens onto that global in
`nocterm-ui`; standard gpui-component button variants and theme colours remain
unchanged. Without the global, the upstream button dimensions apply. An
explicit custom `Size::Size` also keeps its requested size. This keeps compact
product-wide sizing in one place, including buttons supplied by the toolkit.

`src/dock/panel.rs` adds `Panel::tab_accent`, a color the tab bar marks a
panel's tab with; it defaults to `None`, so other panels are unchanged.
`src/dock/tab_panel.rs` passes it to `Tab::accent` in `src/tab/tab.rs`, which
draws a thin stripe along the tab's top edge. Nocterm marks the tabs of a tab
group with the group's color this way, without forking tab rendering or drag
and drop; which tabs form a group stays with the workspace. Remove this patch
when upstream lets a panel style its own tab.

`src/floating.rs` (added) defines `FloatingCards`, an optional application
global with the gap, corner radius and colours of a floating layout, and the
card surface and corner-mask helpers. Nocterm projects its appearance settings
onto it in `nocterm-ui`; without the global every component keeps its upstream
edge-to-edge appearance. With it, `src/dock/tab_panel.rs` draws each tab group
as a card inside half a gap of margin, with a transparent tab bar and a corner
mask beside (never inside) the scrolling content region; `src/dock/dock.rs`
leaves split frames unfilled; and `src/resizable.rs` hides the resting
hairline so the gap is the divider. `src/tab/tab.rs` and `src/tab/tab_bar.rs`
add `TabVariant::Floating`: rounded tabs, concentric with the card, whose
selection fill slides between tabs, and whose group accent is an inset
underline. Remove this patch when upstream offers an equivalent card layout.

`src/dock/tab_panel.rs` and its added `drag_preview.rs` child share tab and
single-panel title presentation between their interactive source and passive
preview. `src/tab/tab.rs` observes its native outer layout box through the added
`bounds.rs` child, preserving borders and close suffixes; its existing tests were
moved unchanged into a child module. Painted bounds and inherited typography replace the fixed 96-by-30
preview; the unchanged `DragPanel` payload receives the actual size and pointer
offset for drop geometry. Tabs retain aliases or rich titles, group accents and
close suffixes. Previews preserve the source bar, card or custom title backdrop
behind transparent content. Floating previews use the existing selected Tab fill because
the TabBar's separate sliding indicator is outside the drag root. No extra
preview frame, padding, opacity or controls are introduced. The child module's
native pointer regressions cover active and inactive default tabs, floating tabs
and single-panel titles.

`src/dock/panel.rs` adds presentation hooks `menu_visible` (default `true`)
and `free_header_content` (default `None`), delegated through `PanelView` and
`PanelHandle`. The extracted `tab_panel/header.rs` suppresses the ellipsis only
when requested and places custom content solely in the unoccupied single-title
area or existing trailing tab-bar space. Tabs, aliases, toolbar controls and
existing drag/drop handlers retain their original ownership. Nocterm's bottom
local terminals use these hooks for direct add/zoom controls and a blank-header
double-click gesture; other panels preserve their original presentation.
Existing tab-group tests were moved unchanged into coherent child modules.
Remove this patch when upstream provides equivalent free-header customization.

`src/message_scroller.rs` exposes the existing virtual list's read-only
`logical_scroll_top` query through `MessageScrollerState`. Nocterm uses the row
index and within-row offset to navigate between chat messages without measuring
or scanning the transcript. Scroll actions, tail following, the scrollbar and the
built-in jump-to-latest control keep their existing ownership. Remove this
forwarder when upstream exposes an equivalent query.


`Cargo.toml` lists `log` under `[package.metadata.cargo-machete] ignored`.
Nocterm runs cargo-machete over the whole repository; the upstream dependency
is kept unchanged so the manifest does not drift from upstream. Remove the
entry when upstream drops or uses the dependency.
