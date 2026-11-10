//! In a hub window each saved server's row says what the hub's agents found there; a server
//! the agents may not sign in to, and every row of other windows, says nothing.
use super::*;
use crate::{AgentAccessState, test_support::ScriptedAgentAccess};

/// A panel listing one saved server, in a hub window when `access` is given.
fn panel_of(
    access: Option<Arc<ScriptedAgentAccess>>,
    cx: &mut TestAppContext,
) -> (gpui_kit::AnyWindowHandle, Profile) {
    let (handle, workspace, _, installed) = match access {
        Some(access) => crate::test_support::workspace_with_agent_access(cx, access),
        None => crate::test_support::workspace(cx),
    };
    let profile = gp("web", None);
    cx.update_window(handle, |_, window, cx| {
        installed.connections().update(cx, |connections, cx| {
            connections
                .save_profile(profile.clone(), cx)
                .now_or_never()
                .unwrap()
                .unwrap();
        });
        let panel = cx
            .new(|cx| ConnectionsPanel::new(installed.clone(), workspace.downgrade(), window, cx));
        workspace.update(cx, |workspace, cx| {
            add_servers(workspace, panel, window, cx)
        });
    })
    .unwrap();
    (handle, profile)
}

fn badge(
    handle: gpui_kit::AnyWindowHandle,
    profile: &Profile,
    cx: &mut TestAppContext,
) -> Option<String> {
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        let id = SharedString::from(format!("connection-agents-{}", profile.id));
        window
            .try_find(id)
            .map(|badge| badge.label().unwrap_or_default().to_owned())
    })
    .unwrap()
}

#[gpui_kit::test]
fn a_row_says_what_the_agents_found_on_its_server(cx: &mut TestAppContext) {
    for (state, label) in [
        (AgentAccessState::Ready, "Agents ✓"),
        (AgentAccessState::NoAccess, "No access"),
        (AgentAccessState::HostKeyChanged, "Host key changed"),
        (AgentAccessState::Reconnect, "Reconnect"),
    ] {
        let access = Arc::new(ScriptedAgentAccess::default());
        *access.state.lock().unwrap() = Some(state);
        let (handle, profile) = panel_of(Some(access), cx);
        assert_eq!(badge(handle, &profile, cx).as_deref(), Some(label));
    }
}

#[gpui_kit::test]
fn a_server_without_agent_access_and_the_window_of_this_computer_show_none(
    cx: &mut TestAppContext,
) {
    let (handle, profile) = panel_of(Some(Arc::new(ScriptedAgentAccess::default())), cx);
    assert_eq!(badge(handle, &profile, cx), None);
    let (handle, profile) = panel_of(None, cx);
    assert_eq!(badge(handle, &profile, cx), None);
}
