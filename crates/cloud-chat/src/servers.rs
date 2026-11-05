//! A chat's servers (ADR 0139) as the agent-chat window sees them: the binding the verified
//! history states, the server prompts run on, and binding and unbinding through the chat's own
//! conversation.
use crate::{CloudChat, CloudChatError};
use gpui::{App, Task, WeakEntity};
use nocterm_agent_chat::conversation::{
    BoundServerState, BoundServerView, ConversationError, ConversationServers, ServerOption,
    ServerProblem,
};
use nocterm_cloud_conversation_model::{ServerAccess, ServerKey, ServerRefusal};
use std::time::Duration;

/// The problem a window shows for `refusal`.
pub(crate) fn problem(refusal: ServerRefusal) -> ServerProblem {
    match refusal {
        ServerRefusal::NoCa => ServerProblem::NoCa,
        ServerRefusal::Untrusted => ServerProblem::Untrusted,
        ServerRefusal::UnknownPort => ServerProblem::UnknownPort,
        ServerRefusal::Servers => ServerProblem::Servers,
        ServerRefusal::Window => ServerProblem::Window,
        ServerRefusal::NotBound => ServerProblem::NotBound,
        ServerRefusal::NotSelectable => ServerProblem::NotSelectable,
    }
}

/// The key a window's id names; ids are the key's hex.
fn key_of(id: &str) -> Option<ServerKey> {
    if id.len() != 32 || !id.is_ascii() {
        return None;
    }
    let mut bytes = [0; 16];
    for (index, byte) in bytes.iter_mut().enumerate() {
        *byte = u8::from_str_radix(id.get(index * 2..index * 2 + 2)?, 16).ok()?;
    }
    Some(ServerKey::from_bytes(bytes))
}

fn state(access: ServerAccess) -> BoundServerState {
    match access {
        ServerAccess::Pending => BoundServerState::Pending,
        ServerAccess::Active { until_ms } => BoundServerState::ActiveUntil(until_ms),
        ServerAccess::Unreachable => BoundServerState::Unreachable,
        ServerAccess::Refused => BoundServerState::Refused,
        ServerAccess::Expired => BoundServerState::Expired,
        ServerAccess::Revoked => BoundServerState::Revoked,
    }
}

fn conversation(error: impl Into<CloudChatError>) -> ConversationError {
    error.into().into_conversation()
}

/// The servers of one chat, captured for a window.
pub(crate) struct CloudServers(pub(crate) WeakEntity<CloudChat>);
impl CloudServers {
    fn chat(&self) -> Result<gpui::Entity<CloudChat>, ConversationError> {
        self.0.upgrade().ok_or(ConversationError::Stale)
    }
}
impl ConversationServers for CloudServers {
    fn bound(&self, cx: &App) -> Vec<BoundServerView> {
        let Ok(chat) = self.chat() else {
            return Vec::new();
        };
        chat.read(cx)
            .transcript()
            .servers()
            .servers
            .iter()
            .map(|server| BoundServerView {
                id: server.server.to_string(),
                label: format!("{}@{}:{}", server.user, server.host, server.port),
                state: state(server.access),
                selectable: server.selectable,
            })
            .collect()
    }
    fn expires_at_ms(&self, cx: &App) -> Option<i64> {
        let chat = self.chat().ok()?;
        chat.read(cx).transcript().servers().expires_at_ms
    }
    fn target(&self, cx: &App) -> Option<String> {
        let chat = self.chat().ok()?;
        chat.read(cx).prompt_target().map(|key| key.to_string())
    }
    fn set_target(&self, id: Option<String>, cx: &mut App) -> Result<(), ConversationError> {
        let key = match id {
            Some(id) => Some(key_of(&id).ok_or(ServerProblem::NotSelectable)?),
            None => None,
        };
        self.chat()?.update(cx, |chat, cx| chat.set_target(key, cx))
    }
    fn options(&self, cx: &mut App) -> Task<Result<Vec<ServerOption>, ConversationError>> {
        let chat = match self.chat() {
            Ok(chat) => chat,
            Err(error) => return Task::ready(Err(error)),
        };
        let listing = chat.read(cx).port().servers();
        cx.spawn(async move |_| {
            let choices = listing.await.map_err(conversation)?;
            Ok(choices
                .into_iter()
                .map(|choice| ServerOption {
                    id: choice.server.to_string(),
                    label: format!("{}@{}", choice.user, choice.host),
                    bindable: choice.port.is_some(),
                    needs_ca_update: choice.needs_ca_update,
                })
                .collect())
        })
    }
    fn bind(
        &self,
        ids: Vec<String>,
        window: Duration,
        cx: &mut App,
    ) -> Task<Result<(), ConversationError>> {
        let chat = match self.chat() {
            Ok(chat) => chat,
            Err(error) => return Task::ready(Err(error)),
        };
        let keys: Option<Vec<ServerKey>> = ids.iter().map(|id| key_of(id)).collect();
        let Some(keys) = keys else {
            return Task::ready(Err(ServerProblem::Untrusted.into()));
        };
        let binding = chat.read(cx).port().bind_servers(keys, window);
        cx.spawn(async move |_| binding.await.map_err(conversation))
    }
    fn unbind(&self, cx: &mut App) -> Task<Result<(), ConversationError>> {
        let chat = match self.chat() {
            Ok(chat) => chat,
            Err(error) => return Task::ready(Err(error)),
        };
        let unbinding = chat.update(cx, |chat, cx| {
            chat.set_target(None, cx).ok();
            chat.port().unbind_servers()
        });
        cx.spawn(async move |_| unbinding.await.map_err(conversation))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_window_names_a_server_by_its_keys_hex_and_nothing_else() {
        let key = ServerKey::from_bytes([0xa5; 16]);
        assert_eq!(key_of(&key.to_string()), Some(key));
        assert_eq!(key_of("a5"), None);
        assert_eq!(key_of(&"zz".repeat(16)), None);
    }
}
