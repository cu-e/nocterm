//! One chat's non-text prompt draft. Image admission and model selection change one revision;
//! source-bound UI completions clear only the revision they actually submitted.
use crate::{CloudChat, CloudChatError};
use gpui::{App, Task, WeakEntity};
use nocterm_agent_chat::conversation::{
    ConversationComposer, ConversationError, ConversationModel, ConversationProvider,
};
use nocterm_ai::images::{ImageCollectionBudget, PromptImage};
use nocterm_cloud_conversation_model::{ImageMime, ModelName};

#[derive(Default)]
pub(crate) struct PromptDraft {
    images: Vec<PromptImage>,
    selected: Option<ConversationModel>,
    revision: u64,
}
impl PromptDraft {
    pub(crate) fn images(&self) -> &[PromptImage] {
        &self.images
    }
    pub(crate) fn bytes(&self) -> usize {
        self.images
            .iter()
            .map(|image| image.bytes().len() + image.encoded_len())
            .sum()
    }
    pub(crate) fn model(&self) -> Option<ModelName> {
        self.selected.as_ref().map(|model| ModelName {
            provider: model.provider.clone(),
            model: model.model.clone(),
        })
    }
    pub(crate) fn prompt_images(&self) -> Vec<nocterm_cloud_conversation_model::PromptImage> {
        self.images
            .iter()
            .map(|image| nocterm_cloud_conversation_model::PromptImage {
                mime: match image.mime_type() {
                    "image/jpeg" => ImageMime::Jpeg,
                    "image/gif" => ImageMime::Gif,
                    "image/webp" => ImageMime::Webp,
                    _ => ImageMime::Png,
                },
                bytes: image.shared_bytes(),
            })
            .collect()
    }
    pub(crate) fn capabilities_changed(
        &mut self,
        choices: Vec<ModelName>,
    ) -> Result<(), CloudChatError> {
        self.advance_revision()?;
        if self.selected.as_ref().is_some_and(|selected| {
            !choices.iter().any(|choice| {
                choice.provider == selected.provider && choice.model == selected.model
            })
        }) {
            self.selected = None;
        }
        Ok(())
    }
    pub(crate) fn advance_revision(&mut self) -> Result<(), CloudChatError> {
        self.revision = self
            .revision
            .checked_add(1)
            .ok_or(nocterm_cloud_conversation_model::CloudConversationError::Capacity)?;
        Ok(())
    }
    pub(crate) fn clear(&mut self) {
        self.images.clear();
        self.selected = None;
        self.revision = self.revision.saturating_add(1);
    }
}

pub(crate) struct CloudComposer(WeakEntity<CloudChat>);
impl CloudComposer {
    pub(crate) fn new(chat: WeakEntity<CloudChat>) -> Self {
        Self(chat)
    }
    fn chat(&self, cx: &App) -> Result<gpui::Entity<CloudChat>, ConversationError> {
        let chat = self.0.upgrade().ok_or(ConversationError::Stale)?;
        chat.read(cx)
            .check_current()
            .map_err(CloudChatError::into_conversation)?;
        Ok(chat)
    }
}
fn model(value: ModelName) -> ConversationModel {
    ConversationModel {
        provider: value.provider,
        model: value.model,
    }
}
impl ConversationComposer for CloudComposer {
    fn can_choose_provider(&self, cx: &App) -> bool {
        self.chat(cx).is_ok_and(|chat| {
            let chat = chat.read(cx);
            chat.port().can_choose_provider()
        })
    }
    fn providers(
        &self,
        cx: &mut App,
    ) -> Task<Result<Vec<ConversationProvider>, ConversationError>> {
        let chat = match self.chat(cx) {
            Ok(chat) => chat,
            Err(error) => return Task::ready(Err(error)),
        };
        let work = chat.read(cx).port().provider_options();
        let original = self.0.clone();
        cx.spawn(async move |cx| {
            let choices = work.await.map_err(provider_error)?;
            original
                .update(cx, |chat, _| chat.check_current())
                .map_err(|_| ConversationError::Stale)?
                .map_err(CloudChatError::into_conversation)?;
            Ok(choices
                .into_iter()
                .map(|choice| ConversationProvider {
                    id: choice.id,
                    label: choice.label,
                })
                .collect())
        })
    }
    fn select_provider(&self, id: String, cx: &mut App) -> Task<Result<(), ConversationError>> {
        let chat = match self.chat(cx) {
            Ok(chat) => chat,
            Err(error) => return Task::ready(Err(error)),
        };
        if provider_busy(chat.read(cx)) {
            return Task::ready(Err(ConversationError::ProviderBusy));
        }
        if !chat.read(cx).port().can_choose_provider() {
            return Task::ready(Err(ConversationError::Unavailable));
        }
        let work = chat.read(cx).port().select_provider(id);
        let original = self.0.clone();
        cx.spawn(async move |cx| {
            work.await.map_err(provider_error)?;
            original
                .update(cx, |chat, _| chat.check_current())
                .map_err(|_| ConversationError::Stale)?
                .map_err(CloudChatError::into_conversation)
        })
    }
    fn revision(&self, cx: &App) -> Result<u64, ConversationError> {
        Ok(self.chat(cx)?.read(cx).draft.revision)
    }
    fn images(&self, cx: &App) -> Vec<PromptImage> {
        self.chat(cx)
            .map(|chat| chat.read(cx).draft.images.clone())
            .unwrap_or_default()
    }
    fn accept_images(
        &self,
        images: Vec<PromptImage>,
        cx: &mut App,
    ) -> Result<(), ConversationError> {
        self.chat(cx)?.update(cx, |chat, cx| {
            if !chat.can_prompt() {
                return Err(ConversationError::Unavailable);
            }
            let mut budget = ImageCollectionBudget::remaining(&chat.draft.images)?;
            for image in &images {
                budget.admit(image.bytes().len())?;
            }
            let incoming: usize = images
                .iter()
                .map(|image| image.bytes().len() + image.encoded_len())
                .sum();
            chat.reserve_draft(incoming, cx)?;
            if !images.is_empty() {
                let revision = chat
                    .draft
                    .revision
                    .checked_add(1)
                    .ok_or(ConversationError::Capacity)?;
                chat.draft.images.extend(images);
                chat.draft.revision = revision;
                cx.notify();
            }
            Ok(())
        })
    }
    fn remove_image(&self, index: usize, cx: &mut App) -> Result<(), ConversationError> {
        self.chat(cx)?.update(cx, |chat, cx| {
            if index < chat.draft.images.len() {
                let revision = chat
                    .draft
                    .revision
                    .checked_add(1)
                    .ok_or(ConversationError::Capacity)?;
                chat.draft.images.remove(index);
                chat.draft.revision = revision;
                cx.notify();
            }
            Ok(())
        })
    }
    fn clear_images(&self, expected_revision: u64, cx: &mut App) -> Result<(), ConversationError> {
        self.chat(cx)?.update(cx, |chat, cx| {
            if chat.draft.revision != expected_revision {
                return Err(ConversationError::Stale);
            }
            if !chat.draft.images.is_empty() {
                let revision = chat
                    .draft
                    .revision
                    .checked_add(1)
                    .ok_or(ConversationError::Capacity)?;
                chat.draft.images.clear();
                chat.draft.revision = revision;
                cx.notify();
            }
            Ok(())
        })
    }
    fn models(&self, cx: &App) -> Vec<ConversationModel> {
        self.chat(cx)
            .map(|chat| {
                chat.read(cx)
                    .port()
                    .models()
                    .into_iter()
                    .map(model)
                    .collect()
            })
            .unwrap_or_default()
    }
    fn selected_model(&self, cx: &App) -> Option<ConversationModel> {
        let chat = self.chat(cx).ok()?;
        effective_model(chat.read(cx)).map(model)
    }
    fn select_model(
        &self,
        selected: ConversationModel,
        cx: &mut App,
    ) -> Result<(), ConversationError> {
        self.chat(cx)?.update(cx, |chat, cx| {
            if !chat.can_prompt() {
                return Err(ConversationError::Unavailable);
            }
            if !chat
                .port()
                .models()
                .into_iter()
                .any(|choice| model(choice) == selected)
            {
                return Err(ConversationError::ModelUnavailable);
            }
            if chat.draft.selected.as_ref() != Some(&selected) {
                let revision = chat
                    .draft
                    .revision
                    .checked_add(1)
                    .ok_or(ConversationError::Capacity)?;
                chat.draft.selected = Some(selected);
                chat.draft.revision = revision;
                cx.notify();
            }
            Ok(())
        })
    }
}

/// The exact model a new command captures, including an implicit current selection.
pub(crate) fn effective_model(chat: &CloudChat) -> Option<ModelName> {
    if !chat.port().can_prompt() {
        return None;
    }
    let choices = chat.port().models();
    let offered = |model: &ModelName| choices.is_empty() || choices.contains(model);
    chat.draft
        .model()
        .filter(offered)
        .or_else(|| {
            chat.transcript()
                .runs()
                .last()
                .map(|run| run.model.clone())
                .filter(offered)
        })
        .or_else(|| chat.port().default_model())
}

fn provider_busy(chat: &CloudChat) -> bool {
    chat.transcript()
        .shown_run()
        .is_some_and(|run| !run.status.finished())
}
fn provider_error(
    error: nocterm_cloud_conversation_model::CloudConversationError,
) -> ConversationError {
    use nocterm_cloud_conversation_model::CloudConversationError as Error;
    match error {
        Error::Stale | Error::Ended => ConversationError::Stale,
        Error::ProviderBusy => ConversationError::ProviderBusy,
        other => ConversationError::ProviderSelection(nocterm_core::error::SharedCause::new(other)),
    }
}
