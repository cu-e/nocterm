//! One `Watch` per open chat: its new items, the text its agent is writing and its settings,
//! told to the chat as session updates until the chat closes.

use std::sync::{Arc, Mutex, PoisonError, Weak};
use std::time::Duration;

use nocterm_ai::model::{
    auth::{PermissionChoice, PermissionEffect, PermissionOutcome, ProviderPermission},
    controls::Change,
    ids::ToolId,
    session::{SessionInfo, SessionNotice, SessionUpdate, TurnActivity},
    tools::{ToolChanges, ToolPatch},
};
use nocterm_ai::{AgentEvent, PermissionResponder};
use nocterm_hub_proto::v2;

use super::connector::{Inner, fresh};
use super::options::ChatOptions;
use super::state::{ChatState, Output};

/// How long a broken stream waits before it is opened again.
const RETRY: Duration = Duration::from_secs(3);

const ALLOW: &str = "allow";
const ALLOW_IN_CHAT: &str = "allow_chat";
const DENY: &str = "deny";

/// The task that follows one chat. It holds the connection weakly: closing the chat or the
/// connection ends it.
pub(super) struct Follow {
    pub(super) inner: Weak<Inner>,
    pub(super) chat: String,
    pub(super) state: Arc<Mutex<ChatState>>,
    pub(super) options: Arc<Mutex<ChatOptions>>,
}

impl Follow {
    /// Follows the chat; `opening` are the questions a reopened chat left open, and `running`
    /// says a run was under way when it was read.
    pub(super) async fn run(self, opening: Vec<Output>, running: bool) {
        let mut first = Some((opening, running));
        loop {
            let Some(inner) = self.inner.upgrade() else {
                return;
            };
            let after = self
                .state
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .last_seq();
            let stream = async {
                let client = fresh(&inner.captured).await.ok()?;
                let request = v2::WatchRequest {
                    chats_after_rev: 0,
                    open: vec![v2::OpenChat {
                        chat_id: self.chat.clone(),
                        after_seq: after,
                    }],
                };
                client
                    .chats()
                    .watch(request)
                    .await
                    .ok()
                    .map(|s| s.into_inner())
            }
            .await;
            drop(inner);
            if let Some(mut stream) = stream {
                if let Some((opening, running)) = first.take() {
                    if running {
                        self.notice(SessionUpdate::Turn(TurnActivity::Started {
                            prompt: Vec::new(),
                        }));
                    }
                    self.perform(opening);
                }
                while let Ok(Some(event)) = stream.message().await {
                    match self.event(event) {
                        Next::Continue => {}
                        Next::Reopen => break,
                        Next::End => return,
                    }
                }
            }
            tokio::time::sleep(RETRY).await;
        }
    }

    fn event(&self, event: v2::WatchEvent) -> Next {
        use v2::watch_event::Event;
        let outputs = match event.event {
            Some(Event::Item(event)) if event.chat_id == self.chat => match &event.item {
                Some(item) => self.state().item(item),
                None => Vec::new(),
            },
            Some(Event::Draft(draft)) if draft.chat_id == self.chat => self.state().draft(&draft),
            Some(Event::Chat(chat)) if chat.id == self.chat => self.settings(&chat),
            Some(Event::ChatDeleted(id)) if id == self.chat => {
                if let Some(done) = self.state().abandon() {
                    let _ = done.send(Err(nocterm_ai::AgentError::Io(
                        "The chat was deleted.".into(),
                    )));
                }
                return Next::End;
            }
            Some(Event::Account(changed))
                if changed.list == i32::from(v2::AccountList::Servers) =>
            {
                self.servers_changed();
                Vec::new()
            }
            Some(Event::Resync(_)) => return Next::Reopen,
            _ => Vec::new(),
        };
        self.perform(outputs);
        Next::Continue
    }

    fn state(&self) -> std::sync::MutexGuard<'_, ChatState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// A changed title, model or set of servers of the chat, told as metadata and options.
    fn settings(&self, chat: &v2::Chat) -> Vec<Output> {
        let mut options = self.options.lock().unwrap_or_else(PoisonError::into_inner);
        let mut outputs = Vec::new();
        if chat.title != options.title() && !chat.title.is_empty() {
            outputs.push(Output::notice(SessionUpdate::SessionInfoUpdate(
                SessionInfo {
                    title: Change::Set(chat.title.clone()),
                    meta: None,
                },
            )));
        }
        let differs = options.differs(chat);
        *options = options.stored(chat);
        if differs && let Some(inner) = self.inner.upgrade() {
            outputs.push(Output::notice(SessionUpdate::ConfigOptionUpdate(
                inner.config(&options),
            )));
        }
        outputs
    }

    /// The account's servers changed: they are listed again and the chat shows them.
    fn servers_changed(&self) {
        let Some(inner) = self.inner.upgrade() else {
            return;
        };
        let options = self.options.clone();
        let chat = self.chat.clone();
        let weak = self.inner.clone();
        inner.handle.spawn(async move {
            let Some(inner) = weak.upgrade() else {
                return;
            };
            if !inner.list_servers().await {
                return;
            }
            let shown = inner.config(&options.lock().unwrap_or_else(PoisonError::into_inner));
            let notice = SessionNotice::new(chat, SessionUpdate::ConfigOptionUpdate(shown));
            let _ = inner.events.try_send(AgentEvent::Session(notice));
        });
    }

    fn perform(&self, outputs: Vec<Output>) {
        for output in outputs {
            match output {
                Output::Notice(update) => self.notice(*update),
                Output::Permission {
                    approval,
                    call,
                    title,
                } => self.ask(approval, call, title),
                Output::Resolve(done, result) => {
                    let _ = done.send(result);
                }
            }
        }
    }

    fn notice(&self, update: SessionUpdate) {
        if let Some(inner) = self.inner.upgrade() {
            let notice = SessionNotice::new(self.chat.clone(), update);
            let _ = inner.events.try_send(AgentEvent::Session(notice));
        }
    }

    /// Asks the person about a tool call and sends the answer to the hub. Any device may answer
    /// first; a later answer here changes nothing.
    fn ask(&self, approval: String, call: String, title: String) {
        let Some(inner) = self.inner.upgrade() else {
            return;
        };
        let (respond, answer) = PermissionResponder::channel();
        let request = ProviderPermission::new(
            self.chat.clone(),
            ToolPatch::new(ToolId::new(call), ToolChanges::default().title(title)),
            vec![
                PermissionChoice::new(ALLOW, "Allow", PermissionEffect::AllowOnce),
                PermissionChoice::new(
                    ALLOW_IN_CHAT,
                    "Allow in this chat",
                    PermissionEffect::AllowRemembered,
                ),
                PermissionChoice::new(DENY, "Deny", PermissionEffect::DenyOnce),
            ],
        );
        if inner
            .events
            .try_send(AgentEvent::Permission { request, respond })
            .is_err()
        {
            return;
        }
        let chat = self.chat.clone();
        let captured = inner.captured.clone();
        inner.handle.spawn(async move {
            let Ok(PermissionOutcome::Selected(choice)) = answer.await else {
                return;
            };
            let decision = match choice.as_str() {
                ALLOW => v2::ApprovalDecision::AllowOnce,
                ALLOW_IN_CHAT => v2::ApprovalDecision::AllowChat,
                _ => v2::ApprovalDecision::Deny,
            };
            let Ok(client) = fresh(&captured).await else {
                return;
            };
            let request = v2::ResolveApprovalRequest {
                chat_id: chat,
                approval_id: approval,
                decision: decision.into(),
            };
            if let Err(status) = client.chats().resolve_approval(request).await {
                tracing::debug!(code = ?status.code(), "the approval was not taken");
            }
        });
    }
}

enum Next {
    Continue,
    Reopen,
    End,
}
