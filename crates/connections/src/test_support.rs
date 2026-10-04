use std::{cell::RefCell, rc::Rc};

use gpui_kit::{AnyWindowHandle, AppContext as _, Entity, TestAppContext, WindowOptions};
use nocterm_ui::{Design, DesignTokens};
use nocterm_workspace::{SessionSpec, Workspace};

use crate::Connections;

pub(crate) type Opened = Rc<RefCell<Vec<SessionSpec>>>;

pub(crate) fn workspace(cx: &mut TestAppContext) -> (AnyWindowHandle, Entity<Workspace>, Opened) {
    let opened = Opened::default();
    let calls = opened.clone();
    let (window, workspace) = cx.update(|cx| {
        // Keep dialog hitboxes stable between rendering and simulated clicks.
        cx.set_reduce_motion(true);
        gpui_kit::init(cx);
        cx.set_global(Design::new(DesignTokens::builtin()));
        crate::init(None, cx);
        gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
            cx.new(|cx| {
                let mut workspace = Workspace::new(window, cx);
                workspace.set_session_opener(move |_, spec, _, _| calls.borrow_mut().push(spec));
                workspace
            })
        })
        .unwrap()
    });
    assert!(cx.read(|cx| {
        Connections::global(cx)
            .read(cx)
            .profiles()
            .iter()
            .next()
            .is_none()
    }));
    (window, workspace, opened)
}
