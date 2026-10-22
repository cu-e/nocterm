//! Panel fixtures interact through rendered controls and native editor actions.
use super::*;
/// Replace literal text through the same focus, selection and text input as a user.
pub(super) fn replace_composer_value(
    focus: FocusHandle,
    text: impl AsRef<str>,
    window: &mut Window,
    cx: &mut App,
) {
    window.focus(&focus, cx);
    window.press("ctrl-a", cx);
    if text.as_ref().is_empty() {
        window.press("backspace", cx);
    } else {
        window.input(text.as_ref(), cx);
    }
}
pub(super) fn new_chat(fixture: &Fixture, cx: &mut TestAppContext) {
    cx.update_window(fixture.handle, |_, window, cx| {
        let focus = fixture.panel.read(cx).composer_focus(cx);
        window.focus(&focus, cx);
        window.render_frame(cx);
        window.click("agent-new-thread", cx);
        window.render_frame(cx);
        window.click("new-agent-codex", cx);
        window.render_frame(cx);
        let thread = fixture.panel.read(cx).current().unwrap();
        thread.update(cx, |thread, cx| thread.request_activation(cx));
    })
    .unwrap();
    cx.run_until_parked();
}
pub(super) fn show_history(f: &Fixture, cx: &mut TestAppContext) {
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        if window.try_find("agent-history-column").is_none() {
            window.click("agent-history", cx);
            window.render_frame(cx);
        }
    })
    .unwrap();
    cx.run_until_parked();
}
pub(super) fn open_queue(f: &Fixture, cx: &mut TestAppContext) {
    cx.update_window(f.handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("queue-toggle", cx);
        window.render_frame(cx);
    })
    .unwrap();
    cx.run_until_parked();
}

pub(super) fn replace_textbox(
    id: impl Into<gpui_kit::ElementId>,
    text: &str,
    window: &mut Window,
    cx: &mut App,
) {
    window.click(id, cx);
    window.press("ctrl-a", cx);
    if text.is_empty() {
        window.press("backspace", cx);
    } else {
        window.input(text, cx);
    }
}
