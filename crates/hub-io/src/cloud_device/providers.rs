//! An own chat's explicit provider choice and the one authoritative model configuration.
//! Selection is a finite account-bound operation; a failed or cancelled attempt reopens the
//! choice, and only a delivered lease installs its complete model list.
use super::{DeviceAccount, DeviceConversation};
use nocterm_cloud_conversation_model::{
    CloudConversationError, ConversationFuture, ModelName, ProviderOption,
};
use nocterm_cloud_device::{open::ModelChoice, provider::Lease};
use std::sync::{Arc, Mutex, PoisonError, Weak};
use tokio::runtime::Handle;

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct ProviderModels {
    pub(crate) default: ModelChoice,
    pub(crate) models: Vec<ModelName>,
}
/// A captured account's personal agents and the credential lease explicitly selected for a chat.
pub(crate) trait ProviderDirectory: Send + Sync {
    fn options(&self) -> ConversationFuture<Vec<ProviderOption>>;
    fn select(
        &self,
        id: String,
        revision: u64,
        chat: Weak<DeviceConversation>,
    ) -> ConversationFuture<ProviderModels>;
}
impl std::fmt::Debug for dyn ProviderDirectory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ProviderDirectory")
    }
}

enum PromptValue {
    Unchosen,
    Choosing,
    Ready(ProviderModels),
}
struct PromptState {
    revision: u64,
    value: PromptValue,
}
pub(crate) struct PromptModels(Mutex<PromptState>);
impl PromptModels {
    pub(crate) fn new(default: Option<ModelChoice>, models: Vec<ModelName>) -> Self {
        Self(Mutex::new(PromptState {
            revision: 0,
            value: default.map_or(PromptValue::Unchosen, |default| {
                PromptValue::Ready(ProviderModels { default, models })
            }),
        }))
    }
    pub(crate) fn revision(&self) -> u64 {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .revision
    }
    fn needs_provider(&self) -> bool {
        !matches!(
            self.0.lock().unwrap_or_else(PoisonError::into_inner).value,
            PromptValue::Ready(_)
        )
    }
    pub(crate) fn can_prompt(&self) -> bool {
        matches!(
            self.0.lock().unwrap_or_else(PoisonError::into_inner).value,
            PromptValue::Ready(_)
        )
    }
    pub(crate) fn default(&self) -> Option<ModelChoice> {
        match &self.0.lock().unwrap_or_else(PoisonError::into_inner).value {
            PromptValue::Ready(models) => Some(models.default.clone()),
            _ => None,
        }
    }
    pub(crate) fn allows(&self, model: &ModelName) -> bool {
        match &self.0.lock().unwrap_or_else(PoisonError::into_inner).value {
            PromptValue::Ready(models) => {
                models.models.contains(model)
                    || (models.models.is_empty()
                        && model.provider == models.default.provider
                        && model.model == models.default.model)
            }
            _ => false,
        }
    }
    pub(crate) fn models(&self) -> Vec<ModelName> {
        match &self.0.lock().unwrap_or_else(PoisonError::into_inner).value {
            PromptValue::Ready(models) => models.models.clone(),
            _ => Vec::new(),
        }
    }
    fn begin(self: &Arc<Self>) -> Result<SelectionAttempt, CloudConversationError> {
        let mut state = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        match state.value {
            PromptValue::Choosing => return Err(CloudConversationError::ProviderSelectionPending),
            PromptValue::Ready(_) => return Err(CloudConversationError::Unsupported),
            PromptValue::Unchosen => {}
        }
        state.value = PromptValue::Choosing;
        Ok(SelectionAttempt {
            models: self.clone(),
            committed: false,
        })
    }
    fn finish(&self, models: ProviderModels) -> Result<bool, CloudConversationError> {
        validate(&models)?;
        let mut state = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if !matches!(state.value, PromptValue::Choosing) {
            return Err(CloudConversationError::Unsupported);
        }
        let revision = state
            .revision
            .checked_add(1)
            .ok_or(CloudConversationError::Capacity)?;
        state.value = PromptValue::Ready(models);
        state.revision = revision;
        Ok(true)
    }
    /// A new own chat retains its requested model only if the actual lease still allows it.
    pub(crate) fn adopt_lease(&self, lease: &Lease) -> Result<bool, CloudConversationError> {
        let mut state = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        let PromptValue::Ready(current) = &state.value else {
            return Err(CloudConversationError::NoModel);
        };
        if current.default.provider != lease.provider
            || !lease.models.contains(&current.default.model)
        {
            return Err(CloudConversationError::NoModel);
        }
        let next = ProviderModels {
            default: current.default.clone(),
            models: lease_models(lease),
        };
        validate(&next)?;
        if current == &next {
            return Ok(false);
        }
        let revision = state
            .revision
            .checked_add(1)
            .ok_or(CloudConversationError::Capacity)?;
        state.value = PromptValue::Ready(next);
        state.revision = revision;
        Ok(true)
    }
    pub(crate) fn refresh_lease(&self, lease: &Lease) -> Result<bool, CloudConversationError> {
        let mut state = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        let PromptValue::Ready(current) = &state.value else {
            return Ok(false);
        };
        let models = lease_models(lease);
        let previous = ModelName {
            provider: current.default.provider.clone(),
            model: current.default.model.clone(),
        };
        let default = if models.contains(&previous) {
            current.default.clone()
        } else {
            ModelChoice {
                provider: lease.provider.clone(),
                model: lease.model.clone(),
            }
        };
        let next = ProviderModels { default, models };
        validate(&next)?;
        if current == &next {
            return Ok(false);
        }
        let revision = state
            .revision
            .checked_add(1)
            .ok_or(CloudConversationError::Capacity)?;
        state.value = PromptValue::Ready(next);
        state.revision = revision;
        Ok(true)
    }
}
fn validate(models: &ProviderModels) -> Result<(), CloudConversationError> {
    if models.models.is_empty()
        || models.models.len() > nocterm_hub_model::agents::limits::MAX_MODELS
        || !models.models.iter().any(|model| {
            model.provider == models.default.provider && model.model == models.default.model
        })
    {
        return Err(CloudConversationError::NoModel);
    }
    Ok(())
}
pub(crate) fn lease_models(lease: &Lease) -> Vec<ModelName> {
    lease
        .models
        .iter()
        .map(|model| ModelName {
            provider: lease.provider.clone(),
            model: model.clone(),
        })
        .collect()
}
struct SelectionAttempt {
    models: Arc<PromptModels>,
    committed: bool,
}
impl SelectionAttempt {
    fn commit(mut self, models: ProviderModels) -> Result<bool, CloudConversationError> {
        let changed = self.models.finish(models)?;
        self.committed = true;
        Ok(changed)
    }
}
impl Drop for SelectionAttempt {
    fn drop(&mut self) {
        if !self.committed {
            let mut state = self.models.0.lock().unwrap_or_else(PoisonError::into_inner);
            if matches!(state.value, PromptValue::Choosing) {
                state.value = PromptValue::Unchosen;
            }
        }
    }
}

pub(crate) struct ProviderSelection {
    account: Arc<DeviceAccount>,
    directory: Arc<dyn ProviderDirectory>,
    conversation: Weak<DeviceConversation>,
    models: Arc<PromptModels>,
    handle: Handle,
}
impl ProviderSelection {
    pub(crate) fn new(
        account: Arc<DeviceAccount>,
        directory: Arc<dyn ProviderDirectory>,
        conversation: Weak<DeviceConversation>,
        models: Arc<PromptModels>,
        handle: Handle,
    ) -> Self {
        Self {
            account,
            directory,
            conversation,
            models,
            handle,
        }
    }
    pub(crate) fn can_choose(&self) -> bool {
        self.models.needs_provider() && self.account.check().is_ok()
    }
    pub(crate) fn options(&self) -> ConversationFuture<Vec<ProviderOption>> {
        if let Err(error) = self.account.check() {
            return Box::pin(async move { Err(stale(error)) });
        }
        let work = self.directory.options();
        let account = self.account.clone();
        Box::pin(async move {
            let options = work.await?;
            account.check().map_err(stale)?;
            Ok(options)
        })
    }
    pub(crate) fn select(&self, id: String) -> ConversationFuture<()> {
        let attempt = match self.models.begin() {
            Ok(attempt) => attempt,
            Err(error) => return Box::pin(async move { Err(error) }),
        };
        let account = self.account.clone();
        let original = self.conversation.clone();
        let directory = self.directory.clone();
        let task = self.handle.spawn(async move {
            account.check().map_err(stale)?;
            let chat = original.upgrade().ok_or(CloudConversationError::Ended)?;
            let revision = chat.provider_revision().await?;
            let selected = directory.select(id, revision, original).await?;
            account.check().map_err(stale)?;
            if attempt.commit(selected)? {
                chat.notify_composer_changed();
            }
            Ok(())
        });
        Box::pin(async move { task.await.map_err(|_| CloudConversationError::Ended)? })
    }
}
fn stale(_: nocterm_hubs_model::HubError) -> CloudConversationError {
    CloudConversationError::Stale
}

/// A provider lease starts only against a freshly witnessed, idle verified conversation.
pub(super) fn admission_revision(
    chat: &nocterm_cloud_device::open::OpenChat,
) -> Result<u64, CloudConversationError> {
    let transcript = chat.transcript();
    if transcript.deleted() {
        return Err(CloudConversationError::Deleted);
    }
    if !matches!(
        chat.freshness(),
        nocterm_cloud_device::reader::Freshness::Current { .. }
    ) {
        return Err(CloudConversationError::Unavailable);
    }
    if transcript.active_run().is_some() {
        return Err(CloudConversationError::ProviderBusy);
    }
    Ok(transcript.control_revision())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn offered() -> ProviderModels {
        ProviderModels {
            default: ModelChoice {
                provider: "own".into(),
                model: "first".into(),
            },
            models: ["first", "second"]
                .into_iter()
                .map(|model| ModelName {
                    provider: "own".into(),
                    model: model.into(),
                })
                .collect(),
        }
    }
    #[tokio::test]
    async fn dropping_a_waiter_does_not_allow_a_second_choice_before_owned_work_finishes() {
        let models = Arc::new(PromptModels::new(None, Vec::new()));
        let attempt = models.begin().unwrap();
        let (release, physical) = tokio::sync::oneshot::channel();
        let (finished, done) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            let _ = physical.await;
            attempt.commit(offered()).unwrap();
            let _ = finished.send(());
        });
        drop(task);
        assert!(models.needs_provider());
        assert!(!models.can_prompt());
        assert!(models.begin().is_err());
        release.send(()).unwrap();
        done.await.unwrap();
        assert!(models.can_prompt());
        assert_eq!(models.models().len(), 2);
        assert!(!models.needs_provider());
    }
    #[test]
    fn an_uncommitted_attempt_reopens_the_choice_and_never_exposes_models() {
        let models = Arc::new(PromptModels::new(None, Vec::new()));
        drop(models.begin().unwrap());
        assert!(models.needs_provider());
        assert!(models.begin().is_ok());
        assert!(models.models().is_empty());
        assert!(!models.can_prompt());
    }
    #[test]
    fn actual_lease_models_replace_the_startup_hint_without_changing_an_unavailable_request() {
        let offered = offered();
        let models = PromptModels::new(Some(offered.default), vec![offered.models[0].clone()]);
        let lease = Lease {
            grant: [7; 16],
            provider: "own".into(),
            model: "first".into(),
            models: vec!["first".into(), "second".into()],
            expires_at_ms: 100,
            renew_at_ms: 50,
        };
        models.adopt_lease(&lease).unwrap();
        assert_eq!(models.models().len(), 2);
        let changed = Lease {
            model: "other".into(),
            models: vec!["other".into()],
            ..lease
        };
        assert!(matches!(
            models.adopt_lease(&changed),
            Err(CloudConversationError::NoModel)
        ));
        assert_eq!(models.default().unwrap().model, "first");
        assert_eq!(models.models().len(), 2);
    }
}
