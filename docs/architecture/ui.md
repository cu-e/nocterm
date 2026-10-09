# UI layer

Part of the [architecture overview](../ARCHITECTURE.md).

UI crates are shared by every feature and depend only on each other, the
domain and the foundation: `nocterm-ui`, `nocterm-workspace` and
`nocterm-keymap`. The workspace's seams are how features meet without depending
on each other ([ADR 0006](../adr/0006-workspace-ports.md)).

## Theme, icons and notices

`nocterm-ui` projects tokens/settings into the standard GPUI Kit theme and
provides shared icons, terminal styling, standalone drag previews and operational
notices. Optional toolkit button metrics are projected from design tokens once;
views retain standard component variants. Notices use the toolkit's window-local
notification list with stable operation keys and recovery actions, published by
operation events rather than rendering. Drag previews capture painted source
bounds and inherited typography because GPUI renders them outside the application
root. Passive source renderers preserve Connections and Explorer row appearance;
drag payloads remain independent of the visual snapshot, including multi-selection.
Connections draw quiet insertion overlays and explicit append targets after every
expanded group's last row. The toolkit shares tab/title presentation with passive
previews and reports the actual source size and pointer offset to dock drop geometry.
Terminal elements reuse the emulator snapshot until output, presentation state,
size or palette changes. Selection, scrolling and search advance a presentation
revision before event subscribers run. Input at an unchanged live screen waits
for shell echo; cursor timing and IME overlays redraw only when their presentation
changes. VT scroll boundaries preserve selection and avoid unnecessary output
notifications, including selection recomputation in vi mode.

## Rendering

Windows incremental painting stays inside the vendored GPUI renderer. Exact
scene comparisons identify conservative damage; a retained image is cleared and
recomposed only within that region, then copied completely to the swap chain.
Terminal and workspace features do not manage GPU textures or dirty rectangles.
Resize, device recovery, atlas content changes and unsupported content force a full
frame. Unsupported partial-clear devices and retained-allocation failures keep
the full-render path. The [renderer patch notes](../../vendor/gpui-pre-windows/NOCTERM_PATCH.md)
describe the resource lifecycle and pixel-equivalence tests.

Linux uses original full rendering by default. Experimental incremental painting
requires `NOCTERM_EXPERIMENTAL_LINUX_RETAINED_RENDERER=1` and stays inside the
pinned WGPU renderer. Supported
surfaces receive a complete copy of a same-format retained image on every
presentation, including unchanged scenes. Partial updates overwrite the damaged
rectangle without blending and replay intersecting batches in order; reopened
path passes restore the scissor. Atlas content revisions and renderer lifecycle
changes invalidate the image. Capability, allocation and bounded-memory
fallbacks preserve the original full renderer, which remains the pixel oracle.
Two bounded snapshots reuse storage; an exact record-pair memo resets for each
comparison. Snapshots rotate only after successful submission with an unchanged
atlas revision, and invalidation releases their storage.
The [WGPU patch notes](../../vendor/gpui-pre-wgpu/NOCTERM_PATCH.md) record the Linux
scope, provenance and tests. Linux scroll-copy is not enabled.
Measured retained scrolling still increased CPU per event by about 3.3–3.8%
on the tested Radeon/Vulkan system, so it is not enabled by default.
The shared GPUI Div scroll handler uses the same snapped and rounded bounds as
prepaint to clamp a wheel offset before comparing and notifying. Events keep
bubbling; fitting containers and scroll boundaries avoid transient invalidation.

## Workspace

`nocterm-workspace` owns window
layout, tabs and sidebar switches through a native DockArea/DockSkin. The
toolkit owns the single pane tree and drag previews; an adapter exposes Items as
dock Panels. Presentation aliases are separate from Item/session titles. Shared
actions operate the same dock for keyboard split/reorder/focus. Tab context-menu
closures snapshot the clicked Item and resolve its current pane order at execution;
closing adjacent/other tabs cannot cross pane boundaries. The full-width workspace
footer owns section switches and status views. Removing the last bottom Item also
removes its Dock instead of merely emptying the tab list. Hiding the local dock
uses native dock visibility, preserving its pane tree, sizes, tab order,
selections, groups and running processes. Every user tab shares one Item registry
and the same close, split, grouping, rename and keyboard operations. Close scopes
resolve the clicked tab's current native pane; Close All affects its dock placement.
Local shell capability is an optional Item contract, so mixed panes and moved local
tabs retain identical ownership. Local focus preserves the last central remote
context for Explorer; native group selection owns the local cwd target. Header
add controls pass an explicit `LocalTerminalTarget::Beside(EntityId)` to the opener,
resolving the live anchor pane when the new shell registers, including moves to
Center. A stale anchor closes only the new shell; registered Items are never
closed by a rejected registration. Native pane queries and empty-region cleanup
belong to the shared workspace docking module. DockItem adapts presentation;
the workspace owns focus subscriptions and active session selection.
Zoom permits tab reordering and grouping: same-group reorders retain zoom, while
accepted topology changes clear it to reveal their result. Hiding Bottom leaves
local tabs moved to Center visible. A region's last tab can be dragged to another
open visible region; the final visible workspace tab and explicit locks remain
protected. Empty noncentral regions disappear after moves without closing Items. An `Item` supplies tab content, focus and an
optional `SessionContext`; a `Panel` supplies sidebar content. Features register
actions rather than making the shell depend on them. `SessionSpec` and a session
opener connect requests from the connections feature to the terminal feature.
Cached session contexts avoid reading an Item reentrantly during its own render;
matching connected contexts also support transfer retry while a utility tab is active.

## Tab groups

Tabs can be grouped. Groups are workspace state the dock knows nothing of:
`tab_groups` holds membership and colors, and after every layout change
`TabGroups::gather` finds the member the user moved and brings the rest of its
group beside it in their previous order, so dragging any member drags the
group, and a tab dropped among a group is moved out of it. Each tab's group
color reaches the tab bar through the vendored `Panel::tab_accent`. A program
opened from a session on the same host joins that session's group; splitting a
tab off takes it out of its group, and a group left with one tab ends.

## Sessions without a tab

Workspace also keeps sessions without a tab. `open_background_session` asks an
installed background opener (Terminal's) for an Item it holds outside the dock;
such sessions appear in `terminals()` marked `background`, can be moved into a
tab with `show_background_session`, and end with `close_background_session`. The
agent opens them for attached saved servers that have no session; a chat closes
those it opened when it closes or no longer attaches the server.

## The active host

Features that run programs on the active host share the workspace's `host`
module: `Host::resolve` picks the session's own connection when it can run
programs and this computer otherwise, and `follow_active_session` reports tab
switches and reconnects.

## Command palette

The command palette (`workspace::command_palette`, Ctrl+Shift+P, Cmd+Shift+P on
macOS) lists no commands of its own. When it opens it asks GPUI for the actions
available where focus is — the focused view's, its ancestors' and application
handlers — keeps nocterm's namespaces (`workspace`, `terminal`, `connections`,
`files`, `agent`, `vault`), names them from the action (`workspace::OpenSettings`
→ "Workspace: Open Settings"), searches their `actions!` documentation as well
and shows the shortcut. The chosen action is dispatched from the view that had
focus, exactly as its shortcut would be, so a feature makes a command available
in the palette by declaring and handling an action, nothing more.

## Layout memory

Widths the user drags resizable columns to, and layout choices, are kept by
`nocterm_ui::LayoutMemory`, a global keyed by name and saved to `layout.json` in
the state directory. The workspace body lays out only its visible columns —
sidebar, tabs, side panel, or mirrored when `SwapSides` put the side panel on
the left — with one resize state per arrangement, and remembers widths per
column rather than per position: a hidden column kept in the resizable group
would hold its stale width and stop its neighbours from growing. The side panel
learns its side through `RightPanel::set_docked_left`; the agent panel puts its
history column on the outer side and remembers that column's width. A side
panel that opens a column of its own asks the workspace to widen it with
`RightPanelEvent::Widen`. Sidebar panels are shown and hidden by their feature's
action (`files::ToggleExplorer`, `connections::ToggleServers`,
`containers::ToggleContainers`) through
`Workspace::toggle_panel_of`, so the workspace binds no feature's panel.

## Key bindings

Key bindings are data. `nocterm-keymap` (UI layer) owns the keymap format, the
merge of the application's defaults (`assets/keymap.toml`) with the user's
`keymap.toml`, which lists only differences (`"none"` unbinds), and installs the
merge in GPUI. Its bindings carry a metadata mark, so a change replaces exactly
them without a restart and leaves the component library's own bindings alone.
It also names actions for people (`humanize`) and decides which are user
commands (`offered`), for the command palette and the keymap page alike.
`nocterm-keymap-ui` is the Keymap settings page, a guest page like the vault's:
a searchable table that records new shortcuts through a keystroke interceptor
(so even bound keys are captured) and edits only through `nocterm-keymap`.
