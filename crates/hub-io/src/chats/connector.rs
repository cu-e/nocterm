//! The hub connector: a hub agent's chats as the sessions of an agent connection.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use futures::future::BoxFuture;
use nocterm_ai::model::{
    controls::ConfigOption,
    diagnostic::ModelError,
    ids::{AuthMethodId, SessionId},
    session::{
        AgentCapabilitiesSummary, PromptOutcome, PromptRequest, RestoreSession, SessionOpen,
        SessionOpened, SetConfigOption, SetMode,
    },
};
use nocterm_ai::{
    AgentCommands, AgentConnection, AgentConnector, AgentError, AgentEvent, AgentInfo,
    CloseSessionOutcome, ConnectError, ConnectRequest,
};
use nocterm_hub_agents_model::HubServer;
use nocterm_hub_client::{ClientError, HubClient};
use nocterm_hub_proto::v2;
use nocterm_hubs_model::HubError;
use tokio::runtime::Handle;

use super::options::{Changed, ChatOptions};
use super::state::ChatState;
use super::watch::Follow;
use crate::classify::classify;
use crate::devices::Captured;
use crate::links::HubLinks;
use crate::pool::Pool;

/// The most history items a reopened chat reads at once.
const HISTORY_PAGE: u32 = 200;
/// The most items a reopened chat shows; older ones stay on the hub.
const MAX_HISTORY: usize = nocterm_ai::history::MAX_SAVED_ENTRIES;

/// Connects chats of hub agents (launches with [`nocterm_ai::agents::HubAgentRef`]).
pub struct HubAgentConnector {
    handle: Handle,
    pool: Arc<Pool>,
}

impl HubAgentConnector {
    pub fn new(links: &HubLinks) -> Self {
        let (handle, pool) = links.agent_parts();
        Self { handle, pool }
    }
}

/// A failed hub call before it is told in the agent port's terms.
pub(super) enum CallFailure {
    Client(ClientError),
    Hub(HubError),
    Refused(String),
}

impl From<tonic::Status> for CallFailure {
    fn from(status: tonic::Status) -> Self {
        Self::Client(ClientError::from(status))
    }
}

impl From<HubError> for CallFailure {
    fn from(error: HubError) -> Self {
        Self::Hub(error)
    }
}

impl CallFailure {
    fn into_agent(self, captured: &Captured) -> AgentError {
        let error = match self {
            Self::Refused(message) => return AgentError::Io(message),
            Self::Client(ClientError::Status {
                hub_code: Some(v2::ErrorCode::LimitReached),
                message,
                ..
            }) => return AgentError::Io(format!("A limit of the hub is reached: {message}")),
            Self::Client(error) => classify(captured.hub.address(), error),
            Self::Hub(error) => error,
        };
        AgentError::Io(error.to_string())
    }
}

fn hub_failure(error: &HubError) -> AgentError {
    AgentError::Io(error.to_string())
}

/// A client of the captured account with a fresh access token.
pub(super) async fn fresh(captured: &Captured) -> Result<HubClient, CallFailure> {
    let client = captured.client.clone();
    captured
        .observed
        .during(client.ensure_fresh())
        .await?
        .map_err(CallFailure::Client)?;
    Ok(client)
}

/// Runs `work` on the adapter's threads with a fresh client of the captured account.
pub(super) fn call<T, F, Fut>(
    handle: &Handle,
    captured: &Arc<Captured>,
    work: F,
) -> BoxFuture<'static, Result<T, AgentError>>
where
    T: Send + 'static,
    F: FnOnce(HubClient) -> Fut + Send + 'static,
    Fut: std::future::Future<Output = Result<T, CallFailure>> + Send + 'static,
{
    let captured = captured.clone();
    let task = crate::task::spawn(handle, async move {
        let outcome = async {
            let client = fresh(&captured).await?;
            captured.observed.during(work(client)).await?
        }
        .await;
        outcome.map_err(|failure| failure.into_agent(&captured))
    });
    Box::pin(async move {
        task.await
            .unwrap_or_else(|| Err(hub_failure(&HubError::Stale)))
    })
}

impl AgentConnector for HubAgentConnector {
    fn connect(
        &self,
        request: ConnectRequest,
    ) -> BoxFuture<'static, Result<AgentConnection, ConnectError>> {
        let Some(agent) = request.launch.hub.clone() else {
            return Box::pin(async {
                Err(AgentError::Unsupported(ModelError::Unsupported(
                    "a launch that is not a hub agent",
                )))
            });
        };
        let captured = match Captured::new(self.pool.clone(), &agent.hub) {
            Ok(captured) => Arc::new(captured),
            Err(error) => return Box::pin(async move { Err(hub_failure(&error)) }),
        };
        let handle = self.handle.clone();
        let name = request.launch.name.clone();
        let wanted = agent.agent.clone();
        let listed = call(&handle, &captured, |client| async move {
            let agents = client.agents().list_agents(v2::Empty {}).await?;
            let agent = agents
                .into_inner()
                .agents
                .into_iter()
                .find(|agent| agent.id == wanted)
                .ok_or_else(|| {
                    CallFailure::Refused("The agent no longer exists on the hub.".into())
                })?;
            // Without the servers a chat still works; it offers none until they are listed.
            let servers = servers_of(&client).await.unwrap_or_default();
            Ok((agent, servers))
        });
        Box::pin(async move {
            let (record, servers) = listed.await?;
            let (events_sender, events) = async_channel::unbounded();
            let commands = Arc::new(HubChatCommands(Arc::new(Inner {
                handle,
                captured,
                agent: record,
                events: events_sender,
                chats: Mutex::default(),
                servers: Mutex::new(servers),
            })));
            Ok(AgentConnection {
                info: AgentInfo {
                    name,
                    version: String::new(),
                    capabilities: AgentCapabilitiesSummary {
                        image_prompts: true,
                        keeps_history: true,
                        own_tools: true,
                    },
                    auth_methods: Vec::new(),
                },
                commands,
                events,
            })
        })
    }
}

pub(super) struct Inner {
    pub(super) handle: Handle,
    pub(super) captured: Arc<Captured>,
    agent: v2::Agent,
    pub(super) events: async_channel::Sender<AgentEvent>,
    chats: Mutex<HashMap<String, OpenChat>>,
    /// The account's servers as last listed: a chat offers to attach each of them.
    servers: Mutex<Vec<HubServer>>,
}

/// The account's servers; a record the hub sent malformed is left out.
async fn servers_of(client: &HubClient) -> Result<Vec<HubServer>, CallFailure> {
    let listed = client.servers().list_servers(v2::Empty {}).await?;
    Ok(listed
        .into_inner()
        .servers
        .into_iter()
        .filter_map(|server| crate::agents::hub_server(server).ok())
        .collect())
}

/// A chat this connection shows. A new chat exists only here until its first message creates
/// it on the hub with the same id; then its `Watch` is followed.
struct OpenChat {
    state: Arc<Mutex<ChatState>>,
    options: Arc<Mutex<ChatOptions>>,
    follow: Option<tokio::task::AbortHandle>,
}

impl Drop for OpenChat {
    fn drop(&mut self) {
        if let Some(follow) = &self.follow {
            follow.abort();
        }
    }
}

/// The commands of one hub agent's connection; every session is a chat of the agent.
struct HubChatCommands(Arc<Inner>);

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Inner {
    fn chats(&self) -> MutexGuard<'_, HashMap<String, OpenChat>> {
        lock(&self.chats)
    }

    /// What `options` shows as the chat's configuration, with the account's servers.
    pub(super) fn config(&self, options: &ChatOptions) -> Vec<ConfigOption> {
        options.config(&lock(&self.servers))
    }

    /// Lists the account's servers again; `false` when the hub did not answer.
    pub(super) async fn list_servers(&self) -> bool {
        let listed = async {
            let client = fresh(&self.captured).await?;
            self.captured.observed.during(servers_of(&client)).await?
        };
        match listed.await {
            Ok(servers) => {
                *lock(&self.servers) = servers;
                true
            }
            Err(_) => false,
        }
    }

    /// Shows `chat` from `state` on, returning its options; it is followed once
    /// [`Self::follow`] is called.
    fn track(&self, chat: &v2::Chat, state: ChatState) -> Vec<ConfigOption> {
        let options = ChatOptions::of(chat, &self.agent.models);
        let shown = self.config(&options);
        self.chats().insert(
            chat.id.clone(),
            OpenChat {
                state: Arc::new(Mutex::new(state)),
                options: Arc::new(Mutex::new(options)),
                follow: None,
            },
        );
        shown
    }

    /// Follows the hub's chat `id`, telling the person what its state left open. Following
    /// twice is harmless.
    fn follow(self: &Arc<Self>, id: &str) {
        let mut chats = self.chats();
        let Some(open) = chats.get_mut(id).filter(|open| open.follow.is_none()) else {
            return;
        };
        let (opening, running) = {
            let state = lock(&open.state);
            (state.open_approvals(), state.running())
        };
        let follow = Follow {
            inner: Arc::downgrade(self),
            chat: id.to_owned(),
            state: open.state.clone(),
            options: open.options.clone(),
        };
        open.follow = Some(
            self.handle
                .spawn(follow.run(opening, running))
                .abort_handle(),
        );
    }

    /// The chat's state, its options, and whether it exists on the hub yet.
    fn open_chat(
        &self,
        chat: &str,
    ) -> Option<(Arc<Mutex<ChatState>>, Arc<Mutex<ChatOptions>>, bool)> {
        self.chats().get(chat).map(|open| {
            (
                open.state.clone(),
                open.options.clone(),
                open.follow.is_some(),
            )
        })
    }
}

impl Inner {
    /// Follows `id` once its first message created it on the hub.
    fn follow_if_created(self: &Arc<Self>, id: &str, created: bool) {
        if created {
            self.follow(id);
        }
    }
}

/// Creates the chat `id` of `agent` on the hub with the model, mode and servers it was shown
/// with. Creating it again (a retried first message) returns it.
async fn create(
    client: &HubClient,
    agent: String,
    id: &str,
    options: ChatOptions,
) -> Result<(), CallFailure> {
    let (model, approval) = options.model();
    let chat = client
        .chats()
        .create_chat(v2::CreateChatRequest {
            agent_id: agent,
            chat_id: id.to_owned(),
            model: model.clone(),
            server_ids: options.servers(),
            ..Default::default()
        })
        .await?
        .into_inner();
    if chat.approval != approval {
        let request = v2::SetChatModelRequest {
            id: id.to_owned(),
            model,
            approval,
        };
        client.chats().set_chat_model(request).await?;
    }
    Ok(())
}

/// Reads the chat's history, oldest first, up to [`MAX_HISTORY`] items.
async fn history(client: &HubClient, chat: &str) -> Result<Vec<v2::ChatItem>, CallFailure> {
    let mut pages = Vec::new();
    let mut before = 0;
    let mut count = 0;
    loop {
        let page = client
            .chats()
            .history(v2::HistoryRequest {
                chat_id: chat.to_owned(),
                before_seq: before,
                limit: HISTORY_PAGE,
            })
            .await?
            .into_inner();
        count += page.items.len();
        let first = page.items.first().map(|item| item.seq);
        let more = page.more;
        pages.push(page.items);
        match first {
            Some(first) if more && count < MAX_HISTORY => before = first,
            _ => break,
        }
    }
    Ok(pages.into_iter().rev().flatten().collect())
}

impl AgentCommands for HubChatCommands {
    /// A new chat is shown with the agent's model and mode at once; its first message creates
    /// it on the hub under the same id, so opening a chat leaves nothing on the hub.
    fn new_session(
        &self,
        _request: SessionOpen,
    ) -> BoxFuture<'static, Result<SessionOpened, AgentError>> {
        let inner = &self.0;
        let chat = v2::Chat {
            id: uuid::Uuid::new_v4().to_string(),
            agent_id: inner.agent.id.clone(),
            model: inner.agent.model.clone(),
            approval: inner.agent.approval,
            ..Default::default()
        };
        let options = inner.track(&chat, ChatState::default());
        Box::pin(async move { Ok(SessionOpened::new(chat.id).configuration(options)) })
    }

    fn restore_session(
        &self,
        request: RestoreSession,
    ) -> BoxFuture<'static, Result<SessionOpened, AgentError>> {
        if request.fork {
            return Box::pin(async {
                Err(AgentError::RestoreUnavailable(
                    "A hub chat cannot be copied.".into(),
                ))
            });
        }
        let inner = self.0.clone();
        let id = request.session.to_string();
        let read = call(&inner.handle, &inner.captured, |client| async move {
            let chat = client
                .chats()
                .get_chat(v2::IdRequest { id: id.clone() })
                .await?
                .into_inner();
            let items = history(&client, &id).await?;
            Ok((chat, items))
        });
        Box::pin(async move {
            let (chat, items) = read.await?;
            let mut state = ChatState::default();
            let updates = items.iter().flat_map(|item| state.replay(item)).collect();
            let options = inner.track(&chat, state);
            inner.follow(&chat.id);
            Ok(SessionOpened::new(chat.id)
                .configuration(options)
                .history(updates))
        })
    }

    fn prompt(
        &self,
        request: PromptRequest,
    ) -> BoxFuture<'static, Result<PromptOutcome, AgentError>> {
        let inner = self.0.clone();
        let chat = request.session.to_string();
        let Some((state, options, created)) = inner.open_chat(&chat) else {
            return Box::pin(async { Err(AgentError::Io("The chat is not open.".into())) });
        };
        let client_message = uuid::Uuid::new_v4().to_string();
        let (done, answer) = futures::channel::oneshot::channel();
        lock(&state).expect(client_message.clone(), done);
        let creation = (!created).then(|| (inner.agent.id.clone(), lock(&options).clone()));
        let id = chat.clone();
        let sent = call(&inner.handle, &inner.captured, move |client| async move {
            if let Some((agent, options)) = creation {
                create(&client, agent, &chat, options).await?;
            }
            let (text, attachment_ids) =
                super::upload::prepare(&client, &chat, request.content).await?;
            let request = v2::SendRequest {
                chat_id: chat,
                client_message_id: client_message,
                text,
                attachment_ids,
            };
            client.chats().send(request).await?;
            Ok(())
        });
        Box::pin(async move {
            let outcome = sent.await;
            // Once created, the chat is followed even when the message did not go.
            inner.follow_if_created(&id, created || outcome.is_ok());
            if let Err(error) = outcome {
                lock(&state).abandon();
                return Err(error);
            }
            answer.await.unwrap_or_else(|_| {
                Err(AgentError::Io(
                    "The chat closed before the agent answered.".into(),
                ))
            })
        })
    }

    fn cancel(&self, session: SessionId) {
        let id = session.to_string();
        let stopping = call(&self.0.handle, &self.0.captured, |client| async move {
            client.chats().stop(v2::IdRequest { id }).await?;
            Ok(())
        });
        self.0.handle.spawn(async move {
            if let Err(error) = stopping.await {
                tracing::warn!(%error, "the hub did not stop the chat's agent");
            }
        });
    }

    fn set_mode(&self, _request: SetMode) -> BoxFuture<'static, Result<(), AgentError>> {
        Box::pin(async {
            Err(AgentError::Unsupported(ModelError::Unsupported(
                "modes of a hub agent",
            )))
        })
    }

    /// Chooses the model or the approval mode, or attaches or detaches a server. A chat its
    /// first message has not created keeps the choice until it does.
    fn set_config_option(
        &self,
        request: SetConfigOption,
    ) -> BoxFuture<'static, Result<Vec<ConfigOption>, AgentError>> {
        let inner = self.0.clone();
        let chat = request.session.to_string();
        let Some((current, created)) = inner
            .chats()
            .get(&chat)
            .map(|open| (open.options.clone(), open.follow.is_some()))
        else {
            return Box::pin(async { Err(AgentError::Io("The chat is not open.".into())) });
        };
        let mut chosen = lock(&current).clone();
        let changed = match chosen.choose(request.config.as_str(), &request.setting) {
            Ok(changed) => changed,
            Err(refused) => return Box::pin(async move { Err(AgentError::Io(refused.0.into())) }),
        };
        if !created {
            // Not on the hub yet: its first message creates it with these.
            let shown = inner.config(&chosen);
            *lock(&current) = chosen;
            return Box::pin(async move { Ok(shown) });
        }
        let saved = call(&inner.handle, &inner.captured, move |client| async move {
            let mut chats = client.chats();
            let stored = match changed {
                Changed::Model => {
                    let (model, approval) = chosen.model();
                    let request = v2::SetChatModelRequest {
                        id: chat,
                        model,
                        approval,
                    };
                    chats.set_chat_model(request).await?
                }
                Changed::Servers => {
                    let request = v2::SetChatServersRequest {
                        id: chat,
                        server_ids: chosen.servers(),
                    };
                    chats.set_chat_servers(request).await?
                }
            };
            Ok(stored.into_inner())
        });
        Box::pin(async move {
            let chat = saved.await?;
            let stored = lock(&current).stored(&chat);
            let shown = inner.config(&stored);
            *lock(&current) = stored;
            Ok(shown)
        })
    }

    fn authenticate(&self, _method: AuthMethodId) -> BoxFuture<'static, Result<(), AgentError>> {
        Box::pin(async { Ok(()) })
    }

    fn close_session(
        &self,
        session: SessionId,
    ) -> BoxFuture<'static, Result<CloseSessionOutcome, AgentError>> {
        self.0.chats().remove(session.as_str());
        Box::pin(async { Ok(CloseSessionOutcome::Closed) })
    }

    fn shutdown(&self) {
        self.0.chats().clear();
        self.0.events.close();
    }
}
