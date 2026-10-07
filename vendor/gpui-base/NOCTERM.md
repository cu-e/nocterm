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
