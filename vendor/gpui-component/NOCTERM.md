# Nocterm's GPUI Component menu patch

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
upstream assertions are removed. Their original paths are
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
