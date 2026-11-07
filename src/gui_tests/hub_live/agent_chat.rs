//! A message typed in a hub window reaches the agent: a chat with a hub agent sends it to the
//! hub at once.
use super::*;
use crate::windows::AppEnvironmentOpener;
use nocterm_hub_agents_model::{AgentDraft, ApprovalMode, HubAgentDirectory as _, ProviderDraft};
use nocterm_hubs_model::Environment;
use nocterm_hubs_ui::{EnvironmentOpener as _, Placement};

fn labels(handle: AnyWindowHandle, cx: &mut TestAppContext) -> Vec<String> {
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        gpui_kit::base::test_support::snapshots(window)
            .into_iter()
            .filter_map(|row| row.label().map(str::to_owned))
            .collect()
    })
    .unwrap()
}

#[gpui_kit::test]
fn a_message_to_a_hub_agent_leaves_the_queue(cx: &mut TestAppContext) {
    let Some(hub) = hub() else { return };
    cx.executor().allow_parking();
    let credentials = tempfile::tempdir().unwrap();
    let links = HubLinks::start(HubLinksConfig {
        credentials_dir: credentials.path().join("hubs"),
        device_name: "gui test".into(),
    })
    .unwrap();
    block_on(links.probe(&hub.address)).expect("the hub answers");
    let (booted, saved) = boot_live(&links, cx);
    let window = open(&booted, cx);
    sign_in_through_the_dialog(window, &hub, &saved, cx);
    click(window, "sign-in-done", cx);

    let agent = create_agent(&links, &hub);
    start_a_chat_with(window, &booted, &hub, &agent, cx);
    focus_composer(window, cx);
    type_text(window, "hello hub", cx);
    press(window, "enter", cx);
    the_message_leaves_the_queue(window, cx);
    the_history_lists_it_with_the_servers_agents(window, cx);
}

/// A provider and an agent of the signed-in account, made on the hub as the web panel makes
/// them.
fn create_agent(links: &Arc<HubLinks>, hub: &Hub) -> nocterm_hub_agents_model::HubAgent {
    let directory = nocterm_hub_io::HubAgentsLink::new(links);
    let provider = block_on(directory.create_provider(
        &hub.id(),
        ProviderDraft {
            kind: nocterm_hub_agents_model::ProviderKind::OpenAi,
            name: "Stub".into(),
            base_url: "https://models.example/v1".into(),
            models: vec!["m1".into()],
            api_key: None,
        },
    ))
    .expect("a provider");
    block_on(directory.create_agent(
        &hub.id(),
        AgentDraft {
            name: "Ops".into(),
            instructions: String::new(),
            provider: Some(provider.id.clone()),
            model: "m1".into(),
            models: vec!["m1".into()],
            approval: ApprovalMode::Auto,
            servers: Vec::new(),
        },
    ))
    .expect("an agent")
}

/// Focuses the composer's text, found by its placeholder as a person finds it.
fn focus_composer(window: AnyWindowHandle, cx: &mut TestAppContext) {
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        let field = gpui_kit::base::test_support::snapshots(window)
            .into_iter()
            .find(|row| row.label() == Some("Ask about your terminals…"))
            .expect("the composer");
        window.click(field.path().last().unwrap().clone(), cx);
    })
    .unwrap();
    cx.run_until_parked();
}

/// The history of a hub window lists the chat on the server's side, not with local agents.
fn the_history_lists_it_with_the_servers_agents(window: AnyWindowHandle, cx: &mut TestAppContext) {
    if !shows(window, "agent-history-list", cx) {
        click(window, "agent-history", cx);
    }
    assert!(
        shows(window, "agent-history-sides", cx),
        "the window shows {:?}",
        labels(window, cx)
    );
    assert!(
        !shows(window, "agent-history-no-match", cx),
        "the chat is listed with the server's agents"
    );
    click(window, "agent-history-local", cx);
    assert!(shows(window, "agent-history-no-match", cx));
}

/// Opens the hub in `window`, shows the chats and starts a chat with `agent`.
fn start_a_chat_with(
    window: AnyWindowHandle,
    booted: &Booted,
    hub: &Hub,
    agent: &nocterm_hub_agents_model::HubAgent,
    cx: &mut TestAppContext,
) {
    let opener = AppEnvironmentOpener::new(booted.clone());
    cx.update_window(window, |_, window, cx| {
        opener
            .open(
                Environment::Hub(hub.id()),
                Placement::ThisWindow,
                window,
                cx,
            )
            .expect("the hub opens");
    })
    .unwrap();
    cx.run_until_parked();
    dispatch(window, nocterm_workspace::ToggleRightPanel, cx);
    let launch =
        nocterm_ai::agents::AgentLaunch::hub_agent(hub.id(), agent.id.to_string(), "Ops".into());
    let item = format!("new-agent-{}", launch.id);
    eventually(window, "the hub agent in the menu", cx, |cx| {
        click(window, "agent-new-thread", cx);
        let listed = shows(window, item.clone(), cx);
        if !listed {
            press(window, "escape", cx);
        }
        listed
    });
    click(window, item.clone(), cx);
}

/// The message goes to the agent: it leaves the queue and opens the transcript.
fn the_message_leaves_the_queue(window: AnyWindowHandle, cx: &mut TestAppContext) {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        show(window, cx);
        if !shows(window, "agent-queue", cx) && shows(window, ("agent-entry", 0usize), cx) {
            break;
        }
        if Instant::now() > deadline {
            panic!(
                "the message stayed queued; the window shows {:?}",
                labels(window, cx)
            );
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}
