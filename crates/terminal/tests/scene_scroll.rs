//! Keep the vendored scroll proof regressions in the ordinary workspace test gate.

extern crate gpui_kit as gpui;

#[expect(
    clippy::too_many_lines,
    reason = "vendored upstream tests keep their shape"
)]
#[path = "../../../vendor/gpui-pre/tests/nocterm_scroll.rs"]
mod tests;
