//! In a hub window the editor of a saved connection offers Agent access through the injected
//! port, connecting with the connection's saved credentials when no session to it is open;
//! elsewhere, and for a connection not saved yet, it offers none.
use super::*;
use crate::test_support::ScriptedAgentAccess;
use gpui_kit::{Context, EntityId};
use nocterm_workspace::{SessionFactory, SessionSpec, Workspace};
use std::{cell::RefCell, rc::Rc, sync::Arc, time::Duration};

/// Opens sessions without a tab and records them; with `connects`, the session comes up at once.
struct BackgroundSessions {
    opened: Rc<RefCell<Vec<SessionSpec>>>,
    access: Arc<ScriptedAgentAccess>,
    connects: bool,
}

impl SessionFactory for BackgroundSessions {
    fn open_background(
        &self,
        _: &mut Workspace,
        spec: SessionSpec,
        _: &mut Window,
        cx: &mut Context<Workspace>,
    ) -> Option<EntityId> {
        self.opened.borrow_mut().push(spec);
        if self.connects {
            self.access.open_session();
        }
        Some(cx.new(|_| ()).entity_id())
    }
}

fn saved(installed: &Installed, cx: &mut gpui_kit::App) -> Profile {
    use futures::FutureExt as _;
    let profile = build(&fields("web.example"), AuthKind::Auto).unwrap();
    installed.connections().update(cx, |connections, cx| {
        connections
            .save_profile(profile.clone(), cx)
            .now_or_never()
            .unwrap()
            .unwrap();
    });
    profile
}

/// The editor of a saved connection over `access`; `None` is the window of this computer.
/// `connects` is how sessions opened without a tab come up.
fn editor_of_saved(
    access: Option<Arc<ScriptedAgentAccess>>,
    connects: bool,
    cx: &mut TestAppContext,
) -> (
    gpui_kit::AnyWindowHandle,
    Profile,
    Rc<RefCell<Vec<SessionSpec>>>,
) {
    let opened = Rc::default();
    let (handle, workspace, _, installed) = match &access {
        Some(access) => crate::test_support::workspace_with_agent_access(cx, access.clone()),
        None => crate::test_support::workspace(cx),
    };
    if let Some(access) = access {
        let sessions = BackgroundSessions {
            opened: Rc::clone(&opened),
            access,
            connects,
        };
        workspace.update(cx, |workspace, _| workspace.set_session_factory(sessions));
    }
    cx.simulate_window_resize(handle, gpui_kit::size(px(1400.), px(1800.)));
    let profile = cx
        .update_window(handle, |_, window, cx| {
            let profile = saved(&installed, cx);
            open_editor(
                installed,
                Some(profile.clone()),
                workspace.downgrade(),
                window,
                cx,
            );
            profile
        })
        .unwrap();
    cx.run_until_parked();
    (handle, profile, opened)
}

fn toggle(handle: gpui_kit::AnyWindowHandle, cx: &mut TestAppContext) {
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("profile-agent-access", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.executor().advance_clock(Duration::from_secs(1));
    cx.run_until_parked();
}

fn shows(handle: gpui_kit::AnyWindowHandle, id: &'static str, cx: &mut TestAppContext) -> bool {
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.try_find(id).is_some()
    })
    .unwrap()
}

#[gpui_kit::test]
fn the_switch_grants_and_withdraws_access_of_the_saved_server(cx: &mut TestAppContext) {
    let access = Arc::new(ScriptedAgentAccess::with_session());
    let (handle, profile, opened) = editor_of_saved(Some(access.clone()), false, cx);
    for _ in 0..2 {
        toggle(handle, cx);
    }
    assert_eq!(access.granted(), vec![true, false]);
    assert!(opened.borrow().is_empty(), "the open session was used");
    let server = access.changes.lock().unwrap()[0].0.clone();
    assert_eq!(server.connection, profile.id.to_string());
    assert_eq!(server.host, "web.example");
    assert_eq!(server.user, profile.target.user);
}

#[gpui_kit::test]
fn without_an_open_session_the_switch_connects_with_the_saved_credentials_first(
    cx: &mut TestAppContext,
) {
    let access = Arc::new(ScriptedAgentAccess::default());
    let (handle, profile, opened) = editor_of_saved(Some(access.clone()), true, cx);
    toggle(handle, cx);
    let opened = opened.borrow();
    assert_eq!(opened.len(), 1, "one session was opened without a tab");
    assert_eq!(opened[0].target, profile.target);
    assert_eq!(opened[0].profile.as_deref(), Some(&*profile.id.to_string()));
    assert_eq!(access.granted(), vec![true]);
    assert!(!shows(handle, "profile-agent-access-problem", cx));
}

#[gpui_kit::test]
fn a_session_that_does_not_come_up_is_said_under_the_switch_and_changes_nothing(
    cx: &mut TestAppContext,
) {
    let access = Arc::new(ScriptedAgentAccess::default());
    let (handle, _, opened) = editor_of_saved(Some(access.clone()), false, cx);
    toggle(handle, cx);
    assert_eq!(opened.borrow().len(), 1);
    assert!(access.granted().is_empty());
    assert!(shows(handle, "profile-agent-access-problem", cx));
}

#[gpui_kit::test]
fn a_refused_change_is_said_under_the_switch(cx: &mut TestAppContext) {
    let access = Arc::new(ScriptedAgentAccess {
        refusal: Some("The hub did not accept the change."),
        ..ScriptedAgentAccess::with_session()
    });
    let (handle, _, _) = editor_of_saved(Some(access), false, cx);
    toggle(handle, cx);
    assert!(shows(handle, "profile-agent-access-problem", cx));
}

#[gpui_kit::test]
fn the_window_of_this_computer_offers_no_agent_access(cx: &mut TestAppContext) {
    let (handle, _, _) = editor_of_saved(None, false, cx);
    assert!(!shows(handle, "profile-agent-access", cx));
}
