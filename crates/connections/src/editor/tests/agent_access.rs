//! In a hub window the editor of a saved connection offers Agent access through the injected
//! port; elsewhere, and for a connection not saved yet, it offers none.
use super::*;
use crate::{AgentAccessRefused, AgentServer, ServerAgentAccess};
use gpui_kit::Task;
use std::sync::{Arc, Mutex};

/// A port that records each change and grants it, or refuses it with `refusal`.
#[derive(Default)]
struct ScriptedAccess {
    granted: Mutex<bool>,
    changes: Mutex<Vec<(AgentServer, bool)>>,
    refusal: Option<&'static str>,
}

impl ServerAgentAccess for ScriptedAccess {
    fn granted(&self, _: &AgentServer, _: &gpui_kit::App) -> bool {
        *self.granted.lock().unwrap()
    }

    fn set(
        &self,
        server: AgentServer,
        granted: bool,
        _: &mut gpui_kit::App,
    ) -> Task<Result<(), AgentAccessRefused>> {
        self.changes.lock().unwrap().push((server, granted));
        match self.refusal {
            Some(reason) => Task::ready(Err(AgentAccessRefused::new(reason))),
            None => {
                *self.granted.lock().unwrap() = granted;
                Task::ready(Ok(()))
            }
        }
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
fn editor_of_saved(
    access: Option<Arc<ScriptedAccess>>,
    cx: &mut TestAppContext,
) -> (gpui_kit::AnyWindowHandle, Profile) {
    let (handle, workspace, _, installed) = match access {
        Some(access) => crate::test_support::workspace_with_agent_access(cx, access),
        None => crate::test_support::workspace(cx),
    };
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
    (handle, profile)
}

#[gpui_kit::test]
fn the_switch_grants_and_withdraws_access_of_the_saved_server(cx: &mut TestAppContext) {
    let access = Arc::new(ScriptedAccess::default());
    let (handle, profile) = editor_of_saved(Some(access.clone()), cx);
    for _ in 0..2 {
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.click("profile-agent-access", cx);
        })
        .unwrap();
        cx.run_until_parked();
    }
    let changes = access.changes.lock().unwrap().clone();
    let granted: Vec<bool> = changes.iter().map(|(_, granted)| *granted).collect();
    assert_eq!(granted, vec![true, false]);
    let server = &changes[0].0;
    assert_eq!(server.connection, profile.id.to_string());
    assert_eq!(server.host, "web.example");
    assert_eq!(server.user, profile.target.user);
}

#[gpui_kit::test]
fn a_refused_change_is_said_under_the_switch(cx: &mut TestAppContext) {
    let access = Arc::new(ScriptedAccess {
        refusal: Some("Connect to the server first."),
        ..ScriptedAccess::default()
    });
    let (handle, _) = editor_of_saved(Some(access), cx);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("profile-agent-access", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("profile-agent-access-problem").is_some());
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_window_of_this_computer_offers_no_agent_access(cx: &mut TestAppContext) {
    let (handle, _) = editor_of_saved(None, cx);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find("profile-agent-access").is_none());
    })
    .unwrap();
}
