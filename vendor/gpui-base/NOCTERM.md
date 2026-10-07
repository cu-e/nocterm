# Nocterm's GPUI Base zoom patch

This vendored fork, including Nocterm's local modifications, is distributed
under Apache-2.0, as declared by its original Cargo manifest. Upstream copyright
notices and LICENSE-APACHE remain in force; the application's PolyForm Perimeter
license does not apply to this copy.

Origin: the crates.io `gpui-base` 0.7.0 source distribution,
[upstream GPUI Kit repository](https://github.com/longbridge/gpui-kit),
commit `0c830f4d257e69fdd17200650533ab4ca9a40cc0`, path `crates/base`.
The published manifest, dependency versions and package version are retained.
This internal Cargo patch is excluded from Nocterm's workspace.

`src/dock/tab_group.rs` and its added `tab_group/zoom.rs` child distinguish
versioned explicit zoom requests from silent dock synchronization. The new
`dock_area/zoom.rs` child reconciles only the latest request from a live cached
group entity. Applying a request never emits another request or changes pending
request revisions. Explicit DockArea commands invalidate pending requests for
all groups and reconcile flags even when the area already has the target state.
This preserves pending cross-group intent while preventing rapid in/out events
from echoing forever and exhausting memory. Actual state changes retain panel
zoomability checks, notifications and deferred panel callbacks.

Regressions in `dock_area/zoom/tests.rs` cover request bursts, cross-group intent,
explicit overrides, removed/replaced groups and callback behavior. Nocterm's
workspace tests also retain the real native header double-click regression.
Remove this patch when upstream offers equivalent safe request reconciliation.

The published crate's `src/text/text_view.rs` unit test refers to the omitted
repository root README. `tests/upstream/README.md` preserves that exact file from
the same pinned commit; only the test include path changes. Existing assertions
and all other upstream test fixtures remain unchanged.

`dock_area/mutations.rs` applies the common accepted-edit policy: reorder within
one group preserves zoom; changed splits and cross-group moves invalidate pending
zoom intent and reveal the resulting panes. Explicit area locks still refuse moves.
`TabGroup` and its renderer context now derive locking solely from explicit
constraints, preserving last-visible and collapsed guards while allowing zoomed
tabs to be reordered or grouped.

`dock_area/visibility.rs` keeps native dock trees when hiding and clears zoom only
when that zoom belongs to the dock being closed. Hosts can opt out of the default
closed Bottom strip through `set_closed_bottom_strip_visible`; Nocterm supplies its
own footer control and requests zero extent. Other consumers retain upstream
presentation. Mutation/visibility regressions accompany the original bounded zoom
queue tests.

`dock_area/drag_policy.rs` offers an opt-in permission to drag a region's last tab
when another open region has a visible panel. Nocterm enables it for unified tabs.
The default is unchanged, hidden or closed regions do not grant permission, and
`tab_group/constraints.rs` keeps that drag permission separate from last-visible
close guards and explicit locks. The former zoom-lock test now asserts the requested
zoomed drag/drop behavior; explicit lock and last-visible regressions remain intact.
New-panel registration can target a group directly, avoiding a transient cross-group
move that would otherwise clear zoom when adding into a nonfirst split pane.
