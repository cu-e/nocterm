//! Keep the vendored scroll proof regressions in the ordinary workspace test gate.

extern crate gpui_kit as gpui;

#[path = "../../../vendor/gpui-pre/tests/nocterm_scroll.rs"]
mod tests;
