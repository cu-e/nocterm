# UI layer

Part of the [architecture overview](../ARCHITECTURE.md).

UI crates are shared by every feature and depend only on each other, the
domain, service and the foundation: `nocterm-ui`, `nocterm-workspace` and
`nocterm-keymap`. The workspace's seams are how features meet without depending
on each other ([ADR 0006](../adr/0006-workspace-ports.md)).

The shared `SessionOptionsEditor` returns `SessionOptionsInputError`, retaining
numeric parse sources and domain override validation. Callers format it when
presenting a form error; malformed numeric text never becomes a saved override.

## Theme, icons and notices

`nocterm-ui` projects tokens/settings into the standard GPUI Kit theme and
accepts an explicitly installed SettingsHandle and returns its retained UiReady witness; it provides shared icons, terminal styling, standalone drag previews and operational
notices. One retained ThemeUi owns catalog, settings and Design paint subscriptions.
Catalog changes resolve effective tokens; published settings resolve and paint directly
when tokens remain unchanged. Design observation paints only, preserving the one-way
projection. Appearance callbacks apply the same supplied settings owner. UI initialization
paints initial supplied policy; live changes require retaining ThemeUi. Terminal styling
and form surfaces use supplied TerminalSettings plus the current paint tokens/theme. Optional toolkit button metrics are projected from design tokens once;
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

## Settings controls

`SettingsTextField` owns native input, a 600 ms debounce, checked admission and
the latest original receipt. Feature-provided typed bindings read and validate
actual settings candidates; UI keeps no feature validators or document cache.
Each distinct flush immediately enters the existing bounded settings queue,
including while an older write is pending. Same waiting literal is deduplicated
only at an unchanged published semantic revision; a later publication admits a
fresh intention before delayed UI completion. Settled dedup uses the current
confirmed value. The field retains one latest waiter; dropping an older waiter
never revokes accepted service work. Matching completion reconciles canonical
text and diagnostics; a newer literal remains intact. Publication failures
remain visible and the same literal can be retried with Enter. Returning to a
prior value is admitted behind a different pending candidate.

Published settings observations retain the original weak field/window. Forms own
narrow redraw subscriptions with each field's membership, so an asynchronous failure
updates the surrounding error even without a settings publication. Removing a field
retires its handle and form observation together. Hosts flush before close/deactivation;
shutdown captures already accepted edits through the existing service. A draft never
flushed before native quit is not an accepted snapshot. No coordinator phase or quit
handler is added ([ADR 0054](../adr/0054-checked-settings-text-receipts.md)).

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

`Workspace` coordinates eight private owners: focus, `WindowSurfaces`, `Chrome`,
sidebar, independent side panel, body, installed ports and registered actions.
`Chrome` owns window menus, the new-tab popover and status views; the
sidebar and side-panel owners control their own visibility and subscriptions.
Immutable observations expose tab identities and native pane order without giving
callers mutable native entities. Transitions commit membership and subscriptions
before the shell emits events or changes focus (ADR 0026).

`WindowSurfaces` retains the outer connection dock. Each workspace is an outer
`ProjectView` Item containing its own `ProjectSurface` with ordinary terminal tabs,
splits and background sessions. Both docks use the existing tab engine. Window
settings, sidebar and right-panel visibility keep their window owners
([ADR 0067](../adr/0067-project-tabs-with-inner-surfaces.md)).

The active outer Item supplies the visible chat ContextId; switching inner tabs
keeps the same workspace history. Source receipts identify the original native
window, outer Item and inner surface. Selection generations prevent a delayed menu
or focus callback from acting after switching tabs. Hidden sessions keep running
and their events update their original surface.

`ProjectTerminalOpen` captures the connected parent once. Its private terminal
provider retains the original SessionSpec and installation. Opening an inner tab
uses the normal local/SSH terminal lifecycle with only the initial folder changed.
Shell options, credentials, proxy and ordered jump hosts stay on that source.
New tabs and reconnect work independently of the original parent tab.

`ExplorerSource` exposes ordinary session file access and the workspace's initial
folder. Local folders use the existing local pane; SSH folders use the remote
pane. Navigation and file operations follow normal host permissions.

Saved workspace lookup includes ordinary central, local and background terminals.
When no matching connected parent exists, `ProjectTerminalResolve` supplies an
ordinary terminal provider from saved server metadata. Profile destination and
ordered jump route must still match. Authentication happens inside the workspace's
first terminal; no separate server tab or parent-connection waiter is created.
Opening callbacks check the current definition and surface membership. Hidden
workspaces do not steal focus. The header and sidebar identify the server using
its configured name and icon, while the outer tab identifies the workspace
([ADR 0070](../adr/0070-original-workspace-connection-recovery.md)).

Chat attachment menus and tool resolution use the same original terminal seat.
Saved-server metadata is available in both General and workspace chats; active
terminals remain limited to that seat's surface. `ChatConnectionOpen` opens an
explicitly selected server in the original surface through the app's ordinary
terminal composition. It checks the live source before creating a terminal,
retains the saved credentials and ordered jump route, and does not select a hidden
workspace. Closed surfaces and foreign windows refuse the operation.

`nocterm-workspace` owns window layout, tabs and sidebar switches through native
DockArea/DockSkin instances. The toolkit owns one pane tree per retained surface
and drag previews; an adapter exposes Items as dock Panels. Presentation aliases
are separate from Item/session titles. Shared
actions operate the same dock for keyboard split/reorder/focus. Tab context-menu
closures capture the original surface/selection and clicked Item, reject stale
selections and resolve that surface's current pane order at execution;
closing adjacent/other tabs cannot cross pane boundaries. The full-width workspace
footer owns section switches and status views. Removing the last bottom Item also
removes its Dock instead of merely emptying the tab list. Hiding the local dock
uses native dock visibility, preserving its pane tree, sizes, tab order,
selections, groups and running processes. Within a surface every tab shares one
Item registry
and the same close, split, grouping, rename and keyboard operations. Close scopes
resolve the clicked tab's current native pane; Close All affects its dock placement.
`LocalTerminal` directory synchronization returns `LocalDirectoryError`, distinguishing
missing or disconnected shells, unsafe paths, unsupported integration, busy prompts,
charset encoding causes and input queue rejection. Consumers format errors at notice
presentation; the workspace retains the selected target on refusal.
Local shell capability is an optional Item contract, so mixed panes and moved local
tabs retain identical ownership. Local focus preserves the last central remote
context for Explorer; native group selection owns the local cwd target. Header
add controls pass an explicit `LocalTerminalTarget::Beside(EntityId)` to the opener,
resolving the live anchor pane when the new shell registers, including moves to
Center. A stale anchor closes only the new shell; registered Items are never
closed by a rejected registration. Native pane queries and empty-region cleanup
belong to the `Tabs` owner and its `DockTopology` capability. DockItem adapts
presentation; each tab entry owns its feature handle, cached session and focus/event
subscriptions. `LocalDockState` owns selection and remembered height, not a second
placement tree.
Zoom permits tab reordering and grouping: same-group reorders retain zoom, while
accepted topology changes clear it to reveal their result. Hiding Bottom leaves
local tabs moved to Center visible. A region's last tab can be dragged to another
open visible region; the final visible tab of a surface and explicit locks remain
protected. Empty noncentral regions disappear after moves without closing Items. An `Item` supplies tab content, focus and an
optional `SessionContext`; a `Panel` supplies sidebar content. Features register
actions rather than making the shell depend on them. `SessionSpec` and a session
opener connect requests from the connections feature to the terminal feature.
Cached session contexts avoid reading an Item reentrantly during its own render;
matching connected contexts also support transfer retry while a utility tab is active.

The narrow terminal ports, `SessionContext` and `ConnectionSummary` are declared by
the `nocterm-workspace-ports` service crate and re-exported by the workspace, so
services can consume them without depending on the window shell
([ADR 0062](../adr/0062-terminal-contracts-below-the-workspace.md)). They return `TerminalError`, retaining charset sources and
concrete capability, prompt, ownership and queue refusals. `open_on_host` returns
`OpenOnHostError`, retaining launch validation and identifying a missing connected
session. Consumers format these causes at notices or agent transcript boundaries.

The Item supplies an immutable `TerminalCapabilities` record with a required state
port and independent optional operations. `ExpectedSession` binds calls to the
workspace Item and connection epoch. `UserPromptToken` also binds the exact secret
prompt. Unsupported operations refuse explicitly; replacement sessions cannot
inherit queued approvals, grants or prompt answers. Reconnect returns its accepted
epoch so every waiter can reject an unrelated subsequent replacement.

## Tab groups

Tabs can be grouped. Groups are workspace state the dock knows nothing of:
`tab_groups` holds membership and colors, and after every layout change
`TabGroups::gather` finds the member the user moved and brings the rest of its
group beside it in their previous order, so dragging any member drags the
group, and a tab dropped among a group is moved out of it. Each tab's group
color reaches the tab bar through the vendored `gpui_kit::component::dock::Panel::tab_accent`. A program
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
an explicitly injected `nocterm_ui::LayoutHandle`, keyed by name and saved to
`layout.json` in the state directory. Its private `nocterm_store::Persisted` entity
keeps changes immediately visible and coalesces pending snapshots into the latest
arrangement. Bootstrap retains that same handle in the workspace dependencies for
every new window. Agent presentation receives the same handle through its supplied `Installed`.
`observe_size` projects only changes to one normalized visible scalar; caller-owned
subscriptions exclude unrelated keys, flags and persistence-only notifications.
Shared history split updates are applied at the next native group rebuild, preserving
its existing entity and subscriptions without another persistence write.
`LayoutMemory` is an initialization namespace returning the explicit handle. It
keeps the existing application quit hook in its initializer and has no global state
or fallback arrangement. Clones share the original persistence participant.
The explicitly injected `ShutdownCoordinator` captures layout before background
drain and shares one deadline across its registered participants (ADR 0013).
Settings also uses `nocterm-store`, with durable publication rather than optimistic
layout projection. Bootstrap registers both settings and layout with the same
coordinator alongside snippets, connection state, agent documents and output
recording.
The workspace body lays out only its visible columns —
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
Edit APIs return `KeymapError`, preserving native keystroke and persistence causes.
A failed save leaves the desired binding installed and reports the failure;
a later successful edit persists the current desired keymap.
The composition root retains a cloneable KeymapHandle around the private owner and
passes it to each page and window. Named edits install owned marked bindings after
releasing the entity borrow, then notify observers, including persistence failure.
The typed shared PersistError is the same original cause returned to the caller;
invalid keystrokes leave the previous state and diagnostic intact. Native loading
and synchronous keymap persistence remain existing UI-layer migration debt.
See [ADR0051](../adr/0051-retained-keymap-installation.md).
It also names actions for people (`humanize`) and decides which are user
commands (`offered`), for the command palette and the keymap page alike.
`nocterm-keymap-ui` is the Keymap settings page, a guest page like the vault's:
a searchable table that records new shortcuts through a keystroke interceptor
(so even bound keys are captured) and edits only through `nocterm-keymap`.

`TerminalStyle::with_settings` projects an explicitly supplied terminal section
through the current design and theme rules. Terminal views use their installed
SettingsHandle; dialogs and settings surfaces capture their original supplied
owner. Their backgrounds retain the same terminal colour and alpha conversion.

Shared UI does not register or read the AI policy schema. Agent runtime registers
its own supplied settings section; standalone settings-page compositions register
that section explicitly before constructing AI controls.

## Shared settings presentation

`SettingsTextField` owns native text, checked immutable admissions and the original
store receipt. `FieldPresentation` retains that field and its weak redraw observation
with the page membership. The small shared syntax helpers parse numbers and JSON;
agent resolution and credential policy remain in the AI domain and Agent page.
`SettingsPageSpec` supports the same plain lazy guest factories and contextual builtin
factories. The latter receive only their original SettingsHandle and per-Item save
feedback. The additive flush operation leaves existing guest lifecycle contracts intact.
