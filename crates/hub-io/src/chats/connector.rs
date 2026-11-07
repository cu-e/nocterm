//! The hub connector: a hub agent's chats as the sessions of an agent connection.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use futures::future::BoxFuture;
use nocterm_ai::model::{
    controls::{ConfigOption, ConfigSetting},
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
use nocterm_hub_client::{ClientError, HubClient};
use nocterm_hub_proto::v2;
use nocterm_hubs_model::HubError;
use tokio::runtime::Handle;

use super::items;
use super::state::ChatState;
use super::watch::{ChatOptions, Follow};
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
            agents
                .into_inner()
                .agents
                .into_iter()
                .find(|agent| agent.id == wanted)
                .ok_or_else(|| {
                    CallFailure::Refused("The agent no longer exists on the hub.".into())
                })
        });
        Box::pin(async move {
            let record = listed.await?;
            let (events_sender, events) = async_channel::unbounded();
            let commands = Arc::new(HubChatCommands(Arc::new(Inner {
                handle,
                captured,
                agent: record,
                events: events_sender,
                chats: Mutex::default(),
            })));
            Ok(AgentConnection {
                info: AgentInfo {
                    name,
                    version: String::new(),
                    capabilities: AgentCapabilitiesSummary {
                        image_prompts: true,
                        keeps_history: true,
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
}

/// A chat this connection follows.
struct OpenChat {
    state: Arc<Mutex<ChatState>>,
    options: Arc<Mutex<ChatOptions>>,
    follow: tokio::task::AbortHandle,
}

impl Drop for OpenChat {
    fn drop(&mut self) {
        self.follow.abort();
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

    fn options(&self, chat: &v2::Chat) -> Vec<ConfigOption> {
        items::options(&self.agent.models, &chat.model, chat.approval)
    }

    /// Follows `chat` from `state` on, telling the person what `state` left open.
    fn open(self: &Arc<Self>, chat: &v2::Chat, state: ChatState) {
        let opening = state.open_approvals();
        let running = state.running();
        let state = Arc::new(Mutex::new(state));
        let options = Arc::new(Mutex::new(ChatOptions::of(chat, &self.agent.models)));
        let follow = Follow {
            inner: Arc::downgrade(self),
            chat: chat.id.clone(),
            state: state.clone(),
            options: options.clone(),
        };
        let task = self.handle.spawn(follow.run(opening, running));
        self.chats().insert(
            chat.id.clone(),
            OpenChat {
                state,
                options,
                follow: task.abort_handle(),
            },
        );
    }

    fn state_of(&self, chat: &str) -> Option<Arc<Mutex<ChatState>>> {
        self.chats().get(chat).map(|open| open.state.clone())
    }
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
    fn new_session(
        &self,
        _request: SessionOpen,
    ) -> BoxFuture<'static, Result<SessionOpened, AgentError>> {
        let inner = self.0.clone();
        let agent = inner.agent.id.clone();
        let created = call(&inner.handle, &inner.captured, |client| async move {
            let request = v2::CreateChatRequest {
                agent_id: agent,
                ..Default::default()
            };
            Ok(client.chats().create_chat(request).await?.into_inner())
        });
        Box::pin(async move {
            let chat = created.await?;
            let options = inner.options(&chat);
            let mut state = ChatState::default();
            state.skip_to(chat.last_seq);
            inner.open(&chat, state);
            Ok(SessionOpened::new(chat.id).configuration(options))
        })
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
            let options = inner.options(&chat);
            inner.open(&chat, state);
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
        let Some(state) = inner.state_of(&chat) else {
            return Box::pin(async { Err(AgentError::Io("The chat is not open.".into())) });
        };
        let client_message = uuid::Uuid::new_v4().to_string();
        let (done, answer) = futures::channel::oneshot::channel();
        lock(&state).expect(client_message.clone(), done);
        let sent = call(&inner.handle, &inner.captured, move |client| async move {
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
            if let Err(error) = sent.await {
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

    fn set_config_option(
        &self,
        request: SetConfigOption,
    ) -> BoxFuture<'static, Result<Vec<ConfigOption>, AgentError>> {
        let inner = self.0.clone();
        let chat = request.session.to_string();
        let Some(current) = inner.chats().get(&chat).map(|open| open.options.clone()) else {
            return Box::pin(async { Err(AgentError::Io("The chat is not open.".into())) });
        };
        let ConfigSetting::Choice(choice) = request.setting else {
            return Box::pin(async {
                Err(AgentError::Unsupported(ModelError::Unsupported(
                    "a setting that is not a choice",
                )))
            });
        };
        let (mut model, mut approval) = lock(&current).wire();
        match request.config.as_str() {
            items::MODEL => model = choice.to_string(),
            items::APPROVAL => match items::approval_of(choice.as_str()) {
                Some(mode) => approval = mode.into(),
                None => {
                    return Box::pin(async {
                        Err(AgentError::Io("Unknown approval mode.".into()))
                    });
                }
            },
            _ => {
                return Box::pin(async {
                    Err(AgentError::Unsupported(ModelError::Unsupported(
                        "an option a hub chat does not have",
                    )))
                });
            }
        }
        let saved = call(&inner.handle, &inner.captured, move |client| async move {
            let request = v2::SetChatModelRequest {
                id: chat,
                model,
                approval,
            };
            Ok(client.chats().set_chat_model(request).await?.into_inner())
        });
        Box::pin(async move {
            let chat = saved.await?;
            *lock(&current) = ChatOptions::of(&chat, &inner.agent.models);
            Ok(inner.options(&chat))
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
