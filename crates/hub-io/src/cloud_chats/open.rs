//! Opening one chat's conversation source once its key material exists.

use std::sync::Arc;
use std::time::Duration;

use futures::FutureExt as _;
use nocterm_cloud_model::projection::ProjectionError;

use super::budget::{ChatBudget, SOURCE_RESIDENT_BYTES};
use super::error::{self, CloudChatError};
use super::setup::{Capability, Collaborators};
use super::wait::until_open;
use crate::cloud::{
    AccountChatMaterial, CloudAccountMaterial, CloudConversationSource, StoredChatKeyGrant,
};

/// Opens the source, waiting up to `patience` for the runner to claim the chat. A `grant` is
/// sent first and again while waiting; a chat that already exists has none.
pub(super) async fn open_source(
    access: &Arc<CloudAccountMaterial>,
    chat: Arc<AccountChatMaterial>,
    capability: &Capability,
    collaborators: &Collaborators,
    budget: &ChatBudget,
    grant: Option<Arc<StoredChatKeyGrant>>,
    patience: Duration,
) -> Result<Arc<CloudConversationSource>, CloudChatError> {
    let publish = move || match grant.clone() {
        Some(grant) => async move { grant.publish().await.map_err(error::material) }.boxed(),
        None => async { Ok(()) }.boxed(),
    };
    let open = || {
        let attempt = (|| {
            let setup = capability.setup(collaborators)?;
            let resident = budget
                .resident(SOURCE_RESIDENT_BYTES)
                .map_err(|_| ProjectionError::Capacity)?;
            let work = budget
                .work(ChatBudget::capture_work_bytes().map_err(|_| ProjectionError::Capacity)?)
                .map_err(|_| ProjectionError::Capacity)?;
            Ok(CloudConversationSource::open(
                access,
                chat.clone(),
                setup,
                resident,
                work,
            ))
        })();
        match attempt {
            Ok(opening) => opening.boxed(),
            Err(error) => async move { Err(error) }.boxed(),
        }
    };
    until_open(&*collaborators.timer, patience, publish, open).await
}
