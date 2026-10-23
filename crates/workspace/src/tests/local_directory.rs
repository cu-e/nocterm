//! The local terminal's directory is announced from its actual cwd, not its title or status.
use gpui_kit::{
    App, Context, FocusHandle, Focusable, SharedString, TestAppContext, Window, div, prelude::*,
};
use std::{cell::Cell, rc::Rc};

struct LocalProbe {
    focus: FocusHandle,
    cwd: Option<std::path::PathBuf>,
    title: SharedString,
}
impl gpui_kit::EventEmitter<crate::ItemEvent> for LocalProbe {}
impl Focusable for LocalProbe {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}
impl Render for LocalProbe {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().track_focus(&self.focus)
    }
}
impl crate::Item for LocalProbe {
    fn tab_title(&self, _: &App) -> SharedString {
        self.title.clone()
    }
}
impl crate::LocalTerminal for LocalProbe {
    fn cwd(&self, _: &App) -> Option<std::path::PathBuf> {
        self.cwd.clone()
    }
    fn change_directory(
        &mut self,
        _: std::path::PathBuf,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Result<(), crate::LocalDirectoryError> {
        Ok(())
    }
}

#[gpui_kit::test]
fn local_directory_event_tracks_actual_cwd_instead_of_title_or_status(cx: &mut TestAppContext) {
    let (handle, workspace) = super::fixture(cx);
    let local = cx
        .update_window(handle, |_, window, cx| {
            let local = cx.new(|cx| LocalProbe {
                focus: cx.focus_handle(),
                cwd: Some("/tmp".into()),
                title: "Shell".into(),
            });
            workspace.update(cx, |w, cx| w.set_local_terminal(local.clone(), window, cx));
            local
        })
        .unwrap();
    cx.run_until_parked();
    let changes = Rc::new(Cell::new(0));
    let seen = changes.clone();
    let _subscription = cx.update(|cx| {
        cx.subscribe(&workspace, move |_, event, _| {
            if *event == crate::WorkspaceEvent::LocalDirectoryChanged {
                seen.set(seen.get() + 1);
            }
        })
    });
    cx.update(|cx| {
        local.update(cx, |local, cx| {
            local.title = "Different title".into();
            cx.emit(crate::ItemEvent::Changed);
        })
    });
    cx.run_until_parked();
    assert_eq!(
        changes.get(),
        0,
        "title/status updates must not reload Explorer cwd"
    );
    cx.update(|cx| {
        local.update(cx, |local, cx| {
            local.cwd = Some("/var".into());
            cx.emit(crate::ItemEvent::Changed);
        })
    });
    cx.run_until_parked();
    assert_eq!(changes.get(), 1);
    cx.update(|cx| local.update(cx, |_, cx| cx.emit(crate::ItemEvent::Changed)));
    cx.run_until_parked();
    assert_eq!(
        changes.get(),
        1,
        "an unchanged cwd must not be emitted again"
    );
    cx.update_window(handle, |_, window, cx| {
        workspace.update(cx, |w, cx| w.close_local_terminal(window, cx))
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(
        changes.get(),
        2,
        "closing retains the directory-clear event"
    );
}
