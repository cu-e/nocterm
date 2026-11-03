//! What a chat may prompt with. The broker stays the budget authority of a managed agent
//! (ADR 0122); these limits only keep one prompt finite.

use std::sync::Arc;

use nocterm_cloud_model::{
    engine::ModelBudget,
    identity::SourceIdentityError,
    inference::ModelSelection,
    projection::{ProjectionError, RecordKind},
    tool::ToolTimer,
};

use crate::cloud::{
    ConversationPromptPolicy, ConversationSetup, ConversationStorage, ConversationWorkers,
};

const TURNS: u32 = 64;
const IO_BYTES: usize = 1024 * 1024;
/// Each model call reserves the budget that remains, so the budget must stay inside the
/// runner's pricing ceiling for one call (the node's default is 4096 output tokens).
const OUTPUT_TOKENS: u64 = 4096;
const SPEND_MICROS: u64 = 5_000_000;
const LIFETIME_MS: u64 = 10 * 60 * 1000;

/// What the chat's window may do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Capability {
    /// Prompts run on the managed seat's default destination with this model.
    Prompting { model: String },
    /// Prompts run on the person's own runner with the key this device leases to it.
    OwnProvider { provider: String, model: String },
    /// History, cancel and delete only.
    Controls,
}

/// The installation-wide collaborators every conversation source needs.
pub(super) struct Collaborators {
    pub(super) workers: Arc<ConversationWorkers>,
    pub(super) storage: Arc<ConversationStorage>,
    pub(super) timer: Arc<dyn ToolTimer>,
}

impl Capability {
    pub(super) fn setup(
        &self,
        collaborators: &Collaborators,
    ) -> Result<ConversationSetup, ProjectionError> {
        let workers = collaborators.workers.clone();
        let timer = collaborators.timer.clone();
        let storage = collaborators.storage.clone();
        let Some(selection) = self.selection() else {
            return ConversationSetup::controls(LIFETIME_MS, workers, timer, storage);
        };
        let budget = ModelBudget::new(TURNS, IO_BYTES, IO_BYTES, OUTPUT_TOKENS)
            .map_err(|_| ProjectionError::InvalidRecord(RecordKind::ChatSetup))?;
        let policy = ConversationPromptPolicy::runner_default(
            selection.map_err(|_| ProjectionError::InvalidRecord(RecordKind::ChatSetup))?,
            budget,
            SPEND_MICROS,
            LIFETIME_MS,
            LIFETIME_MS,
        )?;
        Ok(ConversationSetup::new(policy, workers, timer, storage))
    }

    /// The model a prompt names, or `None` when the chat cannot prompt.
    fn selection(&self) -> Option<Result<ModelSelection, SourceIdentityError>> {
        match self {
            Self::Controls => None,
            Self::Prompting { model } => Some(ModelSelection::managed(model)),
            Self::OwnProvider { provider, model } => Some(ModelSelection::own(provider, model)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_own_provider_chat_prompts_the_provider_the_runner_configured() {
        let own = Capability::OwnProvider {
            provider: "openai".into(),
            model: "gpt-5".into(),
        };
        let selection = own.selection().unwrap().unwrap();
        assert_eq!(selection.provider().as_str(), "openai");
        assert_eq!(
            selection.pricing().as_str(),
            nocterm_cloud_model::inference::OWN_PRICING
        );
        assert!(Capability::Controls.selection().is_none());
    }

    #[test]
    fn the_prompt_limits_are_valid_for_the_policy() {
        let budget = ModelBudget::new(TURNS, IO_BYTES, IO_BYTES, OUTPUT_TOKENS);
        assert!(budget.is_ok());
        assert!(
            ConversationPromptPolicy::runner_default(
                ModelSelection::managed("model-a").unwrap(),
                budget.unwrap(),
                SPEND_MICROS,
                LIFETIME_MS,
                LIFETIME_MS,
            )
            .is_ok()
        );
    }
}
