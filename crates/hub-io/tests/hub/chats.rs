//! A hub chat's servers on a real hub: a server attached before the first message is attached
//! to the chat the message creates, every device reopens the chat with it, and detaching it
//! is stored at once. The agent, provider and server it records are removed again.

use super::{Hub, hub, links, sign_in};
use futures::executor::block_on;
use nocterm_ai::agents::AgentLaunch;
use nocterm_ai::model::content::Content;
use nocterm_ai::model::controls::{ConfigCategory, ConfigKind, ConfigOption, ConfigSetting};
use nocterm_ai::model::ids::SessionId;
use nocterm_ai::model::session::{PromptRequest, RestoreSession, SessionOpen, SetConfigOption};
use nocterm_ai::{AgentCommands, AgentConnector as _, ConnectRequest};
use nocterm_hub_agents_model::{
    AgentDraft, ApprovalMode, HubAgentDirectory as _, HubServers as _, NewServer, ProviderDraft,
    ProviderKind,
};
use nocterm_hub_io::{HubAgentConnector, HubAgentsLink, HubLinks};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// A host key the hub accepts; no server answers with it, and none needs to.
const HOST_KEY: &str =
    "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOMqqnkVzrm0SdG6UOoqKLsabgH5C9okWi0dh2l9GKJl";

fn connect(links: &HubLinks, launch: &AgentLaunch) -> Arc<dyn AgentCommands> {
    let request = ConnectRequest {
        launch: launch.clone(),
        working_directory: std::env::temp_dir(),
        terminal_auth: false,
        sandbox: None,
        resources: Default::default(),
        cancellation: Default::default(),
    };
    block_on(HubAgentConnector::new(links).connect(request))
        .expect("the hub agent connects")
        .commands
}

/// Whether the options show the server flag `id`, and whether it is on.
fn flag(options: &[ConfigOption], id: &str) -> Option<bool> {
    options
        .iter()
        .find(|option| option.id.as_str() == format!("server:{id}"))
        .map(|option| {
            assert_eq!(option.category, Some(ConfigCategory::Server));
            matches!(option.kind, ConfigKind::Boolean(true))
        })
}

/// The chat as another device reopens it, once the hub has created it.
fn reopened(commands: &Arc<dyn AgentCommands>, chat: &SessionId) -> Vec<ConfigOption> {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let restore = RestoreSession {
            session: chat.clone(),
            working_directory: std::env::temp_dir(),
            endpoints: Vec::new(),
            fork: false,
        };
        match block_on(commands.restore_session(restore)) {
            Ok(opened) => return opened.configuration.unwrap_or_default(),
            Err(error) if Instant::now() > deadline => panic!("the chat never opened: {error}"),
            Err(_) => std::thread::sleep(Duration::from_millis(200)),
        }
    }
}

#[test]
fn a_server_attached_before_the_first_message_stays_with_the_chat() {
    let Some(hub) = hub() else { return };
    let dir = tempfile::tempdir().unwrap();
    let links = links(&dir);
    sign_in(&links, &hub).expect("the account signs in");
    let directory = HubAgentsLink::new(&links);
    let (agent, provider, server) = records(&directory, &hub);
    let launch = AgentLaunch::hub_agent(hub.id(), agent.id.to_string(), agent.name.clone());

    let here = connect(&links, &launch);
    let open = SessionOpen {
        working_directory: std::env::temp_dir(),
        endpoints: Vec::new(),
    };
    let opened = block_on(here.new_session(open)).expect("a new chat");
    let id = server.id.to_string();
    assert_eq!(flag(&opened.configuration.unwrap(), &id), Some(false));
    let attach = SetConfigOption {
        session: opened.session.clone(),
        config: format!("server:{id}").into(),
        setting: ConfigSetting::Boolean(true),
    };
    assert_eq!(
        flag(&block_on(here.set_config_option(attach)).unwrap(), &id),
        Some(true)
    );
    // The first message creates the chat; its answer does not matter here.
    let prompt = here.prompt(PromptRequest {
        session: opened.session.clone(),
        content: vec![Content::text("hello")],
    });
    std::thread::spawn(move || drop(block_on(prompt)));

    let elsewhere = connect(&links, &launch);
    assert_eq!(
        flag(&reopened(&elsewhere, &opened.session), &id),
        Some(true)
    );
    let detach = SetConfigOption {
        session: opened.session.clone(),
        config: format!("server:{id}").into(),
        setting: ConfigSetting::Boolean(false),
    };
    assert_eq!(
        flag(&block_on(here.set_config_option(detach)).unwrap(), &id),
        Some(false)
    );
    let again = connect(&links, &launch);
    assert_eq!(flag(&reopened(&again, &opened.session), &id), Some(false));

    block_on(directory.remove_server(&hub.id(), &server.id)).expect("the server is removed");
    block_on(directory.delete_agent(&hub.id(), &agent.id)).expect("the agent is removed");
    block_on(directory.delete_provider(&hub.id(), &provider)).expect("the provider is removed");
    let _ = block_on(nocterm_hubs_model::HubSignIn::sign_out(&*links, &hub.id()));
}

/// An agent with a provider, and a server of the account, as the web panel makes them.
fn records(
    directory: &HubAgentsLink,
    hub: &Hub,
) -> (
    nocterm_hub_agents_model::HubAgent,
    nocterm_hub_agents_model::ProviderId,
    nocterm_hub_agents_model::HubServer,
) {
    let provider = block_on(directory.create_provider(
        &hub.id(),
        ProviderDraft {
            kind: ProviderKind::OpenAi,
            name: "Stub".into(),
            base_url: "https://models.example/v1".into(),
            models: vec!["m1".into()],
            api_key: None,
        },
    ))
    .expect("a provider");
    let agent = block_on(directory.create_agent(
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
    .expect("an agent");
    let server = block_on(directory.add_server(
        &hub.id(),
        NewServer {
            name: "build".into(),
            host: "build.example".into(),
            port: 22,
            user: "ops".into(),
            host_key: HOST_KEY.into(),
        },
    ))
    .expect("a server");
    (agent, provider.id, server)
}
