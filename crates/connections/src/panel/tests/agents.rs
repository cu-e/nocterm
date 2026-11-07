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

#[gpui_kit::test]
fn an_agent_server_saved_elsewhere_is_saved_here_with_its_address_filled_in(
    cx: &mut TestAppContext,
) {
    let access = Arc::new(ScriptedAgentAccess::default());
    access
        .servers
        .lock()
        .unwrap()
        .push(crate::UnsavedAgentServer {
            name: "prod".into(),
            host: "prod.example".into(),
            port: 2222,
            user: "ops".into(),
            state: AgentAccessState::Ready,
        });
    let (handle, _) = panel_of(Some(access), cx);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click(("agent-server", 0usize), cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.press("enter", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(
            window.try_find(("agent-server", 0usize)).is_none(),
            "a saved server is no longer offered"
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn agent_servers_are_offered_only_once_the_accounts_list_has_arrived(cx: &mut TestAppContext) {
    use crate::test_support::replicate;
    use nocterm_connections_model::test_support::MemoryReplica;
    let access = Arc::new(ScriptedAgentAccess::default());
    access
        .servers
        .lock()
        .unwrap()
        .push(crate::UnsavedAgentServer {
            name: "prod".into(),
            host: "prod.example".into(),
            port: 22,
            user: "ops".into(),
            state: AgentAccessState::Ready,
        });
    for (locked, offered) in [(true, false), (false, true)] {
        let (handle, workspace, _, installed) =
            crate::test_support::workspace_with_agent_access(cx, access.clone());
        let replica = Arc::new(MemoryReplica::new());
        replica.set_locked(locked);
        replicate(&installed, replica, cx);
        cx.update_window(handle, |_, window, cx| {
            let panel = cx.new(|cx| {
                ConnectionsPanel::new(installed.clone(), workspace.downgrade(), window, cx)
            });
            workspace.update(cx, |workspace, cx| {
                add_servers(workspace, panel, window, cx)
            });
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert_eq!(
                window.try_find(("agent-server", 0usize)).is_some(),
                offered,
                "locked: {locked}"
            );
        })
        .unwrap();
    }
}
