//! What a chat may prompt with. The broker stays the budget authority of a managed agent
//! (ADR 0122); these limits only keep one prompt finite.

use std::sync::Arc;

use nocterm_cloud_model::{
    engine::ModelBudget, inference::ModelSelection, projection::ProjectionError, tool::ToolTimer,
};

use crate::cloud::{
    ConversationPromptPolicy, ConversationSetup, ConversationStorage, ConversationWorkers,
};

const TURNS: u32 = 64;
const IO_BYTES: usize = 1024 * 1024;
const OUTPUT_TOKENS: u64 = 64 * 1024;
const SPEND_MICROS: u64 = 5_000_000;
const LIFETIME_MS: u64 = 10 * 60 * 1000;

/// What the chat's window may do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Capability {
    /// Prompts run on the managed seat's default destination with this model.
    Prompting { model: String },
    /// History, cancel and delete only: a plain chat has no device provider yet.
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
        match self {
            Self::Controls => ConversationSetup::controls(LIFETIME_MS, workers, timer, storage),
            Self::Prompting { model } => {
                let budget = ModelBudget::new(TURNS, IO_BYTES, IO_BYTES, OUTPUT_TOKENS)
                    .map_err(|_| ProjectionError::InvalidRecord)?;
                let policy = ConversationPromptPolicy::runner_default(
                    ModelSelection::managed(model).map_err(|_| ProjectionError::InvalidRecord)?,
                    budget,
                    SPEND_MICROS,
                    LIFETIME_MS,
                    LIFETIME_MS,
                )?;
                Ok(ConversationSetup::new(policy, workers, timer, storage))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
