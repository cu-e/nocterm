//! One installation's retained conversations and independent window observers.
use crate::{CloudChat, CloudChatError, CloudConversation};
use gpui::{App, AppContext as _, Context, Entity, EntityId, Subscription, WeakEntity};
use nocterm_agent_chat::conversation::{
    ConversationActions, ConversationError, ConversationSource,
};
use nocterm_agent_runtime::history_admission::HistoryAdmissionHandle;
use nocterm_ai::conversation::ConversationView;
use nocterm_cloud_model::projection::{ProjectionError, ports::CapturedConversationSource};
use std::{collections::BTreeSet, rc::Rc, sync::Arc};

// Metadata bounds are independent of the stricter shared payload/byte budget.
// In particular, hidden retained commands do not gain extra payload capacity.
const MAX_CONVERSATIONS: usize = 128;
const MAX_OBSERVERS: usize = 256;

/// Bootstrap creates this once using the installation's Local history admission
/// and passes clones to every window. There is no account-global service lookup.
#[derive(Clone)]
pub struct CloudChatInstallation(Entity<ConversationRegistry>);
impl CloudChatInstallation {
    pub fn new(admission: HistoryAdmissionHandle, cx: &mut App) -> Self {
        Self(cx.new(|_| ConversationRegistry {
            admission,
            chats: Vec::new(),
            observers: 0,
        }))
    }

    /// The returned renderer source owns one observer. Sharing its Rc shares
    /// that observer; a second call creates an independent window observer.
    /// Existing views always keep their original captured native source.
    pub fn open(
        &self,
        source: Arc<dyn CapturedConversationSource>,
        cx: &mut App,
    ) -> Result<Rc<dyn ConversationSource>, CloudChatError> {
        self.0.update(cx, |registry, cx| registry.open(source, cx))
    }

    /// Release quiet detached metadata. Pending commands and cache workers stay
    /// owned; their actual allocation permits continue to govern admission.
    pub fn release_idle(&self, cx: &mut App) {
        self.0.update(cx, |registry, cx| registry.release_idle(cx));
    }
}

struct ConversationRegistry {
    admission: HistoryAdmissionHandle,
    chats: Vec<RegisteredChat>,
    observers: usize,
}
struct RegisteredChat {
    chat: Entity<CloudChat>,
    observers: BTreeSet<EntityId>,
    shown: BTreeSet<EntityId>,
}
impl ConversationRegistry {
    fn open(
        &mut self,
        source: Arc<dyn CapturedConversationSource>,
        cx: &mut Context<Self>,
    ) -> Result<Rc<dyn ConversationSource>, CloudChatError> {
        source.check_current()?;
        self.release_idle(cx);
        if self.observers >= MAX_OBSERVERS {
            return Err(ProjectionError::Capacity.into());
        }
        let retained = self.chats.iter().position(|entry| {
            let chat = entry.chat.read(cx);
            chat.source.binding() == source.binding() && chat.check_current().is_ok()
        });
        let index = match retained {
            Some(index) => index,
            None => {
                if self.chats.len() >= MAX_CONVERSATIONS {
                    return Err(ProjectionError::Capacity.into());
                }
                let chat = CloudChat::open(source, self.admission.clone(), cx)?;
                self.chats.push(RegisteredChat {
                    chat,
                    observers: BTreeSet::new(),
                    shown: BTreeSet::new(),
                });
                self.chats.len() - 1
            }
        };
        let chat = self.chats[index].chat.clone();
        // Reopening one window must resume observation even if another retained
        // observer is hidden. It does not mutate the source of the shared chat.
        chat.update(cx, |chat, cx| chat.set_shown(true, cx))?;
        let registry = cx.weak_entity();
        let observer = cx.new(|cx| {
            let release = cx.on_release(|observer: &mut ConversationObserver, cx| {
                observer.release(cx);
            });
            ConversationObserver {
                delegate: CloudConversation::new(&chat),
                chat,
                registry,
                id: cx.entity_id(),
                _release: release,
            }
        });
        let entry = &mut self.chats[index];
        entry.observers.insert(observer.entity_id());
        entry.shown.insert(observer.entity_id());
        self.observers += 1;
        Ok(Rc::new(ObservedConversation(observer)))
    }

    fn visibility(
        &mut self,
        chat: EntityId,
        observer: EntityId,
        shown: bool,
        cx: &mut Context<Self>,
    ) -> Result<(), ConversationError> {
        let entry = self
            .chats
            .iter_mut()
            .find(|entry| entry.chat.entity_id() == chat)
            .ok_or(ConversationError::Stale)?;
        if !entry.observers.contains(&observer) {
            return Err(ConversationError::Stale);
        }
        if shown {
            entry.shown.insert(observer);
        } else {
            entry.shown.remove(&observer);
        }
        entry
            .chat
            .update(cx, |chat, cx| chat.set_shown(!entry.shown.is_empty(), cx))
            .map_err(CloudChatError::into_conversation)
    }

    fn release(&mut self, chat: EntityId, observer: EntityId, cx: &mut Context<Self>) {
        if let Some(entry) = self
            .chats
            .iter_mut()
            .find(|entry| entry.chat.entity_id() == chat)
            && entry.observers.remove(&observer)
        {
            self.observers -= 1;
            entry.shown.remove(&observer);
            // Source retirement may already have made visibility unavailable.
            // Releasing an observer never retries or sends any command.
            let _ = entry
                .chat
                .update(cx, |chat, cx| chat.set_shown(!entry.shown.is_empty(), cx));
        }
        self.release_idle(cx);
    }

    fn release_idle(&mut self, cx: &App) {
        self.chats.retain(|entry| {
            if !entry.observers.is_empty() {
                return true;
            }
            let chat = entry.chat.read(cx);
            chat.check_current().is_ok() && !chat.can_evict()
        });
    }
}

/// Owns its original chat but only weakly references the registry. Its GPUI
/// release callback runs with an App, including when a window is destroyed.
struct ConversationObserver {
    delegate: CloudConversation,
    chat: Entity<CloudChat>,
    registry: WeakEntity<ConversationRegistry>,
    id: EntityId,
    _release: Subscription,
}
impl ConversationObserver {
    fn set_shown(&mut self, shown: bool, cx: &mut Context<Self>) -> Result<(), ConversationError> {
        self.chat
            .read(cx)
            .check_current()
            .map_err(CloudChatError::into_conversation)?;
        self.registry
            .update(cx, |registry, cx| {
                registry.visibility(self.chat.entity_id(), self.id, shown, cx)
            })
            .map_err(|_| ConversationError::Stale)??;
        Ok(())
    }

    fn release(&self, cx: &mut App) {
        let _ = self.registry.update(cx, |registry, cx| {
            registry.release(self.chat.entity_id(), self.id, cx);
        });
    }
}

struct ObservedConversation(Entity<ConversationObserver>);
impl ConversationSource for ObservedConversation {
    fn identity(&self) -> EntityId {
        self.0.entity_id()
    }
    fn tool_approvals(
        &self,
        cx: &mut App,
    ) -> Result<
        Vec<Rc<dyn nocterm_agent_chat::conversation::ConversationToolApproval>>,
        ConversationError,
    > {
        let delegate = self.0.read(cx).delegate.clone();
        delegate.tool_approvals(cx)
    }
    fn with_view(
        &self,
        cx: &App,
        visitor: &mut dyn for<'a> FnMut(ConversationView<'a>),
    ) -> Result<(), ConversationError> {
        self.0.read(cx).delegate.with_view(cx, visitor)
    }

    fn observe(
        &self,
        cx: &mut App,
        changed: Rc<dyn Fn(&mut App)>,
    ) -> Result<Subscription, ConversationError> {
        let chat = self.0.read(cx).chat.clone();
        CloudConversation::new(&chat).observe(cx, changed)
    }

    fn set_shown(&self, shown: bool, cx: &mut App) -> Result<(), ConversationError> {
        self.0
            .update(cx, |observer, cx| observer.set_shown(shown, cx))
    }

    fn capture_actions(&self, cx: &App) -> Result<Rc<dyn ConversationActions>, ConversationError> {
        self.0.read(cx).delegate.capture_actions(cx)
    }
}
