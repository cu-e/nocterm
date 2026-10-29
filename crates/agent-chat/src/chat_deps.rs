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
    local_environment::ChatScope,
    metadata::ChatMetadataSource,
    storage::{AgentStorageError, require_local_scope},
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
        self.runtime().check_current().is_ok() && self.ai(cx).enabled
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
    pub fn new_chat(&self, agent: String) -> Result<SavedChat, AgentStorageError> {
        self.runtime().check_current()?;
        let scope = self.runtime().chat_scope()?;
        Ok(self.frame_chat(agent, scope))
    }
    fn frame_chat(&self, agent: String, scope: ChatScope) -> SavedChat {
        let identity = self.inner.metadata.new_identity();
        SavedChat::for_environment(agent, identity.id, identity.created, scope)
    }
    pub(crate) fn new_chat_for_origin(
        &self,
        agent: String,
        origin: &nocterm_ai::execution_origin::ChatKey,
    ) -> SavedChat {
        self.frame_chat(agent, origin.chat_scope())
    }
    pub fn scope(&self) -> ContextId {
        self.runtime().scope()
    }
    pub fn chat_scope(&self) -> Result<ChatScope, AgentStorageError> {
        self.runtime().chat_scope()
    }
    pub(crate) fn require_local_scope(&self, actual: &ChatScope) -> Result<(), AgentStorageError> {
        self.runtime().check_current()?;
        require_local_scope(&self.chat_scope()?, actual)
    }
    fn reserve(
        &self,
        key: nocterm_ai::execution_origin::ChatKey,
        replacing: Option<EntityId>,
        cx: &mut App,
    ) -> Result<(DocumentClaim, Self), ChatOpenError> {
        self.require_local_scope(&key.chat_scope())?;
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
        self.require_local_scope(&key.chat_scope())?;
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
        build: impl FnOnce(
            Self,
            crate::thread::local_payloads::LocalPayloadAllocation,
            &mut Context<AgentThread>,
        ) -> AgentThread,
    ) -> Result<Entity<AgentThread>, ChatOpenError> {
        let allocation = self.reserve_payload(cx)?;
        self.build_allocated(claim, deps, allocation, cx, build)
    }
    fn reserve_payload(
        &self,
        cx: &mut App,
    ) -> Result<crate::thread::local_payloads::LocalPayloadAllocation, ChatOpenError> {
        let allocations = self.runtime().local_history_allocations(cx);
        let resident = allocations
            .reserve_resident(None, cx)
            .map_err(|error| ChatOpenError::Storage(error.into_storage()))?;
        Ok((allocations, resident))
    }
    fn build_allocated(
        &self,
        claim: DocumentClaim,
        deps: Self,
        allocation: crate::thread::local_payloads::LocalPayloadAllocation,
        cx: &mut App,
        build: impl FnOnce(
            Self,
            crate::thread::local_payloads::LocalPayloadAllocation,
            &mut Context<AgentThread>,
        ) -> AgentThread,
    ) -> Result<Entity<AgentThread>, ChatOpenError> {
        let thread = cx.new(|cx| build(deps, allocation, cx));
        self.runtime()
            .commit_document(claim, crate::thread::client(&thread), cx)?;
        Ok(thread)
    }
    fn build_claimed(
        &self,
        key: nocterm_ai::execution_origin::ChatKey,
        replacing: Option<EntityId>,
        cx: &mut App,
        build: impl FnOnce(
            Self,
            crate::thread::local_payloads::LocalPayloadAllocation,
            &mut Context<AgentThread>,
        ) -> AgentThread,
    ) -> Result<Entity<AgentThread>, ChatOpenError> {
        let (claim, deps) = self.reserve(key, replacing, cx)?;
        self.build_committed(claim, deps, cx, build)
    }
    pub(crate) fn create_chat(
        &self,
        agent: String,
        cx: &mut App,
    ) -> Result<Entity<AgentThread>, ChatOpenError> {
        let chat = self.new_chat(agent)?;
        chat.validate_identity().map_err(AgentStorageError::from)?;
        let key = nocterm_ai::execution_origin::ChatKey::from_header(&chat.header);
        self.build_claimed(key, None, cx, |deps, allocation, cx| {
            let mut thread = AgentThread::new(chat, deps, allocation, cx);
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
        self.build_claimed(
            data.key().clone(),
            Some(expected),
            cx,
            |deps, allocation, cx| AgentThread::restarted(data, deps, allocation, cx),
        )
    }
    pub(crate) fn fork_chat(
        &self,
        fork: Fork,
        cx: &mut App,
    ) -> Result<Entity<AgentThread>, ChatOpenError> {
        let fork = fork.prepared().map_err(AgentStorageError::from)?;
        let (claim, deps) = self.reserve(fork.key(), None, cx)?;
        self.build_committed(claim, deps, cx, |deps, allocation, cx| {
            AgentThread::forked(fork, deps, allocation, cx)
        })
    }
    /// Reserve exact ownership before constructing an entity or starting work.
    pub fn restore_chat(
        &self,
        chat: SavedChat,
        cx: &mut App,
    ) -> Result<Entity<AgentThread>, ChatOpenError> {
        self.require_local_scope(&chat.header.chat_scope())?;
        chat.validate_identity().map_err(AgentStorageError::from)?;
        let key = nocterm_ai::execution_origin::ChatKey::from_header(&chat.header);
        let chat = nocterm_ai::history::prepare_owned(chat).map_err(AgentStorageError::from)?;
        let (claim, deps) = self.reserve(key, None, cx)?;
        self.runtime()
            .admit_claimed_history(&claim, chat.resident_bytes(), cx)?;
        self.build_committed(claim, deps, cx, |deps, allocation, cx| {
            AgentThread::restored(chat, deps, allocation, cx)
        })
    }
    /// Admit catalog metadata and ownership before exposing a saved chat.
    pub fn restore_archive(
        &self,
        row: CatalogRow,
        cx: &mut App,
    ) -> Result<Entity<AgentThread>, ChatOpenError> {
        let summary = row.summary();
        self.require_local_scope(&summary.chat_scope())?;
        summary.validate().map_err(AgentStorageError::from)?;
        let key = nocterm_ai::execution_origin::ChatKey::from_summary(summary);
        let (claim, deps) = self.reserve(key, None, cx)?;
        self.runtime().admit_claimed_history(&claim, 0, cx)?;
        let allocation = self.reserve_payload(cx)?;
        let prepared = self
            .runtime()
            .take_prepared_chat(&claim, allocation.1.clone(), cx)?;
        match prepared {
            Some(prepared) => {
                let built = prepared.map(|chat| {
                    self.build_allocated(claim, deps, allocation, cx, |deps, allocation, cx| {
                        AgentThread::restored(chat, deps, allocation, cx)
                    })
                });
                built.value().clone()
            }
            None => self.build_allocated(claim, deps, allocation, cx, |deps, allocation, cx| {
                AgentThread::archived(row, deps, allocation, cx)
            }),
        }
    }
}
