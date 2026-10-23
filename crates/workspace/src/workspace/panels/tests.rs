//! Cached panel content and its chrome have independent invalidations.
use crate::test_support::TestPanel;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    App, Context, FocusHandle, Focusable, SharedString, TestAppContext, Window, div, prelude::*,
};
use std::{cell::Cell, rc::Rc};

#[gpui_kit::test]
fn cached_panel_skips_workspace_redraw_but_updates_its_content_and_badge(cx: &mut TestAppContext) {
    let (handle, workspace) = crate::workspace::tests::fixture(cx);
    let renders = Rc::new(Cell::new(0));
    let observed = Rc::new(Cell::new(0));
    let panel = cx
        .update_window(handle, |_, window, cx| {
            let panel = TestPanel::builder("probe")
                .title("Probe")
                .text("First")
                .render_counter(renders.clone())
                .build(cx);
            workspace.update(cx, |w, cx| {
                w.add_panel(panel.clone(), cx);
                w.activate_panel(0, window, cx);
            });
            window.render_frame(cx);
            panel
        })
        .unwrap();
    // Registration/activation effects are flushed after the initial draw.
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| window.draw(cx).clear(cx))
        .unwrap();
    cx.run_until_parked();
    let count = renders.get();
    let log = observed.clone();
    let _subscription = cx.update(|cx| {
        cx.observe(&workspace, move |_, _| {
            log.set(log.get() + 1);
        })
    });
    cx.update(|cx| workspace.update(cx, |_, cx| cx.notify()));
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.draw(cx).clear(cx);
    })
    .unwrap();
    assert_eq!(
        renders.get(),
        count,
        "workspace notification should reuse the panel subtree"
    );
    let before = observed.get();
    cx.update(|cx| panel.update(cx, |panel, cx| panel.set_text("Second", cx)));
    cx.run_until_parked();
    assert_eq!(
        observed.get(),
        before,
        "content-only notifications do not affect workspace chrome"
    );
    cx.update_window(handle, |_, window, cx| {
        window.draw(cx).clear(cx);
    })
    .unwrap();
    assert!(renders.get() > count, "changed panel content must repaint");
    cx.update(|cx| panel.update(cx, |panel, cx| panel.set_badge(Some("1".into()), cx)));
    cx.run_until_parked();
    assert!(
        observed.get() > before,
        "changed badge must update the footer"
    );
}

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
    let (handle, workspace) = crate::workspace::tests::fixture(cx);
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
