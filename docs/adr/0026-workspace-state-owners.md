# 0026 — Workspace state owners and explicit layout dependency

Date: 2026-10-11
Status: accepted

Workspace coordinated tab membership, native placement, menus, panels, background
sessions and layout through 28 fields and implementations in 17 files. File-size
limits did not bound responsibility or protect cross-field transitions. Tests
mutating its registry and dock entities made ownership changes difficult to verify.

Workspace now holds nine private collaborators: its focus handle, Tabs, Chrome,
BackgroundSessions, Sidebar, RightPanelSlot, Body, WorkspacePorts and ActionRegistry.
Owners expose concrete transitions and immutable observations. They provide no
Deref, mutable parent projection, native entity mutation handle or generic callback
that rewrites Workspace state. Root implementations stay in construction,
composition, interaction and queries; owner modules contain their own types.

Tabs owns seven private fields, including membership entries, groups, central
selection, last announced session and command target, native topology, and local
selection. Each entry owns its feature handle, cached session, dock adapter,
optional local capability, cwd and two subscriptions. DockTopology retains the
single native DockArea and its subscriptions. LocalDockState retains only local
selection, remembered height and the captured add-control origin. Membership and
session caches do not duplicate native placement, pane order or region visibility.

Closing detaches registry membership, group membership and item subscriptions
before invoking the feature close hook. Typed transition effects distinguish
before-focus and after-focus events, and the shell applies them after the owner
commits. Session comparisons preserve the filesystem and executor Arc identities.
Local command focus preserves central remote context. A captured stale menu target
never falls back to another tab. Background promotion retains the original feature
entity and replaces its background subscription with tab subscriptions.

Chrome owns status views, WindowMenus and NewTabControl. WindowMenus retains its
builder, bar, focus and captured target. It constructs an AppMenuBar through the
native constructor and sets its menus explicitly without modifying process-global
menu state. Sidebar, right-panel state, installed session/directory ports and action
registrations each have their own private owner. Body owns arrangement-specific
resize entities and the explicit layout handle. Maximized panel measurements and
widen requests do not persist transient window widths.

LayoutMemory initialization returns a cloneable LayoutHandle over the same
optimistic Persisted entity already registered with ShutdownCoordinator. The
existing compatibility global retains that entity for agent history-column width;
no second writer, queue, participant, global or quit handler is installed. Workspace
Deps carries the handle into Body. Bootstrap retains and clones these dependencies
for every new window. Standalone fixtures create an explicit in-memory handle;
application fixtures use the real bootstrapped handle.

Tests drive public shell actions and observations, feature events, and native drag,
resize, zoom and close gestures. They do not modify private Workspace state or use
a public mutable dock projection. The architecture inventory must shrink for the
parent field count, implementation spread and removed complexity exceptions.

Alternatives rejected: spreading the parent's methods into further files, grouping
public fields behind pass-through structs, retaining a second placement model, or
using the legacy layout global as the new owner's dependency source. These leave
invariants and dependencies controlled by callers rather than by state owners.
