//! What every chat needs from its host, without views.
//!
//! [`ChatDeps`] carries the runtime, agent directory, settings, chat
//! metadata, the window seat and the sign-in terminal. Views keep their own
//! collaborators (images, layout) in the panel host.
use crate::thread::{AgentThread, Fork, RestartData};
use crate::{runtime::RuntimeHandle, seat::Seat, sign_in::SignInTerminal};
use gpui::{App, AppContext as _, Context, Entity, EntityId};
use nocterm_agent_directory::AgentsHandle;
use nocterm_agent_runtime::{CatalogRow, ChatOpenError, DocumentClaim, DocumentClaimOutcome};
use nocterm_ai::{
    history::{ContextId, SavedChat},
    metadata::ChatMetadataSource,
    storage::{AgentStorageError, require_chat_scope},
};
use nocterm_store::SettingsHandle;
use std::{rc::Rc, sync::Arc};

#[derive(Clone)]
pub struct ChatDeps {
    inner: Rc<Deps>,
}
struct Deps {
    runtime: RuntimeHandle,
    agents: AgentsHandle,
    settings: SettingsHandle,
    metadata: Arc<dyn ChatMetadataSource>,
    seat: Seat,
    sign_in: Option<Rc<dyn SignInTerminal>>,
}
impl ChatDeps {
    pub fn new(
        runtime: RuntimeHandle,
        agents: AgentsHandle,
        settings: SettingsHandle,
        metadata: Arc<dyn ChatMetadataSource>,
        seat: Seat,
        sign_in: Option<Rc<dyn SignInTerminal>>,
    ) -> Self {
        Self {
            inner: Rc::new(Deps {
                runtime,
                agents,
                settings,
                metadata,
                seat,
                sign_in,
            }),
        }
    }
    pub fn runtime(&self) -> &RuntimeHandle {
        &self.inner.runtime
    }
    pub fn agents(&self) -> &AgentsHandle {
        &self.inner.agents
    }
    pub fn settings(&self) -> &SettingsHandle {
        &self.inner.settings
    }
    pub fn ai<'a>(&self, cx: &'a App) -> &'a nocterm_ai::AiSettings {
        self.settings().setting::<nocterm_ai::AiSettings>(cx)
    }
    pub fn enabled(&self, cx: &App) -> bool {
        self.ai(cx).enabled
    }
    /// What chats may do with the window.
    pub fn seat(&self) -> &Seat {
        &self.inner.seat
    }
    /// Runs sign-in terminals in this window, when the host can.
    pub fn sign_in(&self) -> Option<&Rc<dyn SignInTerminal>> {
        self.inner.sign_in.as_ref()
    }
    pub fn timestamp(&self) -> u64 {
        self.inner.metadata.timestamp()
    }
    pub fn new_chat(&self, agent: String) -> SavedChat {
        let identity = self.inner.metadata.new_identity();
        SavedChat::for_scope(agent, identity.id, identity.created, self.scope())
    }
    pub fn scope(&self) -> ContextId {
        self.runtime().scope()
    }
    pub(crate) fn require_scope(&self, actual: ContextId) -> Result<(), AgentStorageError> {
        require_chat_scope(self.scope(), actual)
    }
    fn reserve(
        &self,
        key: nocterm_ai::execution_origin::ChatKey,
        replacing: Option<EntityId>,
        cx: &mut App,
    ) -> Result<(DocumentClaim, Self), ChatOpenError> {
        self.require_scope(key.scope())?;
        // Seat.id is the original root window entity, not an inner terminal surface.
        let window = self.seat().id();
        let outcome = match replacing {
            Some(owner) => self
                .runtime()
                .replace_document(key.id().into(), owner, window, cx)?,
            None => self.runtime().claim_document(key.id().into(), window, cx)?,
        };
        let claim = match outcome {
            DocumentClaimOutcome::Owned(claim) => claim,
            DocumentClaimOutcome::AlreadyOpen { window } => {
                return Err(ChatOpenError::AlreadyOpen { key, window });
            }
            DocumentClaimOutcome::Deleted => return Err(ChatOpenError::Deleted(key)),
        };
        let deps = self.for_chat(claim.key())?;
        Ok((claim, deps))
    }
    fn for_chat(
        &self,
        key: &nocterm_ai::execution_origin::ChatKey,
    ) -> Result<Self, AgentStorageError> {
        self.require_scope(key.scope())?;
        Ok(Self {
            inner: Rc::new(Deps {
                runtime: self.inner.runtime.clone(),
                agents: self.inner.agents.clone(),
                settings: self.inner.settings.clone(),
                metadata: self.inner.metadata.clone(),
                seat: self.inner.seat.for_chat(key)?,
                sign_in: self.inner.sign_in.clone(),
            }),
        })
    }
    fn build_committed(
        &self,
        claim: DocumentClaim,
        deps: Self,
        cx: &mut App,
        build: impl FnOnce(Self, &mut Context<AgentThread>) -> AgentThread,
    ) -> Result<Entity<AgentThread>, ChatOpenError> {
        let thread = cx.new(|cx| build(deps, cx));
        self.runtime()
            .commit_document(claim, crate::thread::client(&thread), cx)?;
        Ok(thread)
    }
    fn build_claimed(
        &self,
        key: nocterm_ai::execution_origin::ChatKey,
        replacing: Option<EntityId>,
        cx: &mut App,
        build: impl FnOnce(Self, &mut Context<AgentThread>) -> AgentThread,
    ) -> Result<Entity<AgentThread>, ChatOpenError> {
        let (claim, deps) = self.reserve(key, replacing, cx)?;
        self.build_committed(claim, deps, cx, build)
    }
    pub(crate) fn create_chat(
        &self,
        agent: String,
        cx: &mut App,
    ) -> Result<Entity<AgentThread>, ChatOpenError> {
        let chat = self.new_chat(agent);
        chat.validate_identity().map_err(AgentStorageError::from)?;
        let key =
            nocterm_ai::execution_origin::ChatKey::new(chat.header.scope, chat.header.id.clone());
        self.build_claimed(key, None, cx, |deps, cx| {
            let mut thread = AgentThread::new(chat, deps, cx);
            thread.preview_config(cx);
            thread
        })
    }
    pub(crate) fn restart_chat(
        &self,
        data: RestartData,
        expected: EntityId,
        cx: &mut App,
    ) -> Result<Entity<AgentThread>, ChatOpenError> {
        self.build_claimed(data.key().clone(), Some(expected), cx, |deps, cx| {
            AgentThread::restarted(data, deps, cx)
        })
    }
    pub(crate) fn fork_chat(
        &self,
        fork: Fork,
        cx: &mut App,
    ) -> Result<Entity<AgentThread>, ChatOpenError> {
        let fork = fork.prepared().map_err(AgentStorageError::from)?;
        let (claim, deps) = self.reserve(fork.key(), None, cx)?;
        self.runtime()
            .admit_claimed_history(&claim, fork.resident_bytes(), cx)?;
        self.build_committed(claim, deps, cx, |deps, cx| {
            AgentThread::forked(fork, deps, cx)
        })
    }
    /// Reserve exact ownership before constructing an entity or starting work.
    pub fn restore_chat(
        &self,
        chat: SavedChat,
        cx: &mut App,
    ) -> Result<Entity<AgentThread>, ChatOpenError> {
        self.require_scope(chat.header.scope)?;
        chat.validate_identity().map_err(AgentStorageError::from)?;
        let key =
            nocterm_ai::execution_origin::ChatKey::new(chat.header.scope, chat.header.id.clone());
        let chat = nocterm_ai::history::prepare(chat).map_err(AgentStorageError::from)?;
        let (claim, deps) = self.reserve(key, None, cx)?;
        self.runtime()
            .admit_claimed_history(&claim, chat.resident_bytes(), cx)?;
        self.build_committed(claim, deps, cx, |deps, cx| {
            AgentThread::restored(chat, deps, cx)
        })
    }
    /// Admit catalog metadata and ownership before exposing a saved chat.
    pub fn restore_archive(
        &self,
        row: CatalogRow,
        cx: &mut App,
    ) -> Result<Entity<AgentThread>, ChatOpenError> {
        let summary = row.summary();
        self.require_scope(summary.scope)?;
        summary.validate().map_err(AgentStorageError::from)?;
        let key = nocterm_ai::execution_origin::ChatKey::new(summary.scope, summary.id.clone());
        let (claim, deps) = self.reserve(key, None, cx)?;
        self.runtime().admit_claimed_history(&claim, 0, cx)?;
        let prepared = self.runtime().take_prepared_chat(&claim, cx)?;
        self.build_committed(claim, deps, cx, |deps, cx| match prepared {
            Some(chat) => AgentThread::restored(chat, deps, cx),
            None => AgentThread::archived(row, deps, cx),
        })
    }
}
