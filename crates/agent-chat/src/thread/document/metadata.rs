//! Small durable projections share queued payloads and literal drafts, never transcript media.
use super::super::{Composer, config::ChatConfig};
use nocterm_ai::{
    history::{SavedAttachment, SavedPrompt},
    session_config::Choices,
};
use std::{path::PathBuf, sync::Arc};

#[derive(Clone)]
pub(super) struct Identity {
    agent: String,
    key: nocterm_ai::execution_origin::ChatKey,
    wire_version: u32,
}
impl Identity {
    pub(super) fn new(
        agent: String,
        key: nocterm_ai::execution_origin::ChatKey,
        wire_version: u32,
    ) -> Self {
        Self {
            agent,
            key,
            wire_version,
        }
    }
    pub(super) fn wire_version(&self) -> u32 {
        self.wire_version
    }
    pub(super) fn agent(&self) -> &str {
        &self.agent
    }
    pub(super) fn key(&self) -> &nocterm_ai::execution_origin::ChatKey {
        &self.key
    }
    pub(super) fn id(&self) -> &str {
        self.key.id()
    }
}
#[derive(Clone)]
pub(super) struct Metadata {
    name: Option<String>,
    pinned: bool,
    updated: u64,
    pending_history: bool,
}
impl Metadata {
    pub(super) fn new(updated: u64) -> Self {
        Self {
            name: None,
            pinned: false,
            updated,
            pending_history: false,
        }
    }
    pub(super) fn restored(
        name: Option<String>,
        pinned: bool,
        updated: u64,
        pending_history: bool,
    ) -> Self {
        Self {
            name,
            pinned,
            updated,
            pending_history,
        }
    }
    pub(super) fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }
    pub(super) fn pinned(&self) -> bool {
        self.pinned
    }
    pub(super) fn updated(&self) -> u64 {
        self.updated
    }
    pub(super) fn pending_history(&self) -> bool {
        self.pending_history
    }
    pub(super) fn timestamp(&mut self, now: u64) {
        self.updated = now;
    }
    pub(super) fn rename(&mut self, name: Option<String>) -> bool {
        let name = name
            .map(|name| name.trim().to_owned())
            .filter(|name| !name.is_empty());
        if self.name == name {
            return false;
        }
        self.name = name;
        true
    }
    pub(super) fn pin(&mut self, pinned: bool) -> bool {
        if self.pinned == pinned {
            return false;
        }
        self.pinned = pinned;
        true
    }
    pub(super) fn pending(&mut self, pending: bool) -> bool {
        if self.pending_history == pending {
            return false;
        }
        self.pending_history = pending;
        true
    }
}
#[derive(Clone, Default, PartialEq, Eq)]
pub(crate) struct Resume {
    session: Option<String>,
    workdir: Option<PathBuf>,
}
impl Resume {
    pub(crate) fn new(session: Option<String>, workdir: Option<PathBuf>) -> Self {
        Self { session, workdir }
    }
}
pub(crate) struct Inputs<'a> {
    composer: &'a Composer,
    config: &'a ChatConfig,
    attachments: Vec<SavedAttachment>,
    resume: Resume,
}
impl<'a> Inputs<'a> {
    pub(crate) fn new(
        composer: &'a Composer,
        config: &'a ChatConfig,
        attachments: Vec<SavedAttachment>,
        resume: Resume,
    ) -> Self {
        Self {
            composer,
            config,
            attachments,
            resume,
        }
    }
    pub(super) fn metadata(&self, chat: &mut nocterm_ai::history::SavedChat) {
        chat.draft = self.composer.draft().cloned();
        chat.attachments = self.attachments.to_vec();
        chat.model = self.config.model();
        chat.config = self.config.choices().clone();
        chat.mode = self.config.mode_choice().map(ToString::to_string);
        chat.session_id = self.resume.session.clone();
        chat.workdir = self.resume.workdir.clone();
    }
    pub(super) fn queue(&self) -> Vec<Arc<SavedPrompt>> {
        self.composer
            .queue()
            .iter()
            .map(|prompt| prompt.saved().clone())
            .collect()
    }
    pub(super) fn has_content(&self) -> bool {
        !self.composer.is_empty()
    }
}
#[derive(Clone, Default)]
pub(super) struct SemanticInputs {
    initialized: bool,
    model: Option<String>,
    choices: Choices,
    mode: Option<String>,
    draft: Option<Arc<String>>,
    attachments: Vec<SavedAttachment>,
    queue: Vec<Arc<SavedPrompt>>,
    resume: Resume,
}
impl SemanticInputs {
    pub(super) fn initialized(&self) -> bool {
        self.initialized
    }
    pub(super) fn begin_hydration(&mut self) {
        self.initialized = false;
    }
    pub(super) fn matches(&self, input: &Inputs<'_>) -> bool {
        self.model == input.config.model()
            && self.choices == *input.config.choices()
            && self.mode.as_deref() == input.config.mode_choice().map(|mode| mode.as_str())
            && self.draft.as_deref() == input.composer.draft()
            && self.attachments == input.attachments
            && self.queue.len() == input.composer.queue().len()
            && self
                .queue
                .iter()
                .zip(input.composer.queue())
                .all(|(saved, queued)| {
                    Arc::ptr_eq(saved, queued.saved()) || **saved == **queued.saved()
                })
            && self.resume == input.resume
    }
    pub(super) fn remember(&mut self, input: &Inputs<'_>) {
        self.initialized = true;
        self.model = input.config.model();
        self.choices = input.config.choices().clone();
        self.mode = input.config.mode_choice().map(ToString::to_string);
        self.draft = input.composer.shared_draft();
        self.attachments = input.attachments.to_vec();
        self.queue = input.queue();
        self.resume = input.resume.clone();
    }
}
