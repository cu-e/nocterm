//! Creation-only installation authority, separate from existing-environment management.
use crate::{EnvironmentCommands, EnvironmentFuture, LanguageSelection, OwnedContainer};
use std::sync::Arc;

pub trait CreatedWorkspaceEnvironment: Send + Sync + 'static {
    fn install_created(
        &self,
        commands: Arc<dyn EnvironmentCommands>,
        owned: OwnedContainer,
        selections: Vec<LanguageSelection>,
    ) -> EnvironmentFuture<()>;
}
