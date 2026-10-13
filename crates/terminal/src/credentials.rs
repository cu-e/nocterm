//! Prompts answered from, and remembered in, a [`CredentialStore`].
use gpui_kit::{App, Global, Task};
use nocterm_session::{CredentialId, CredentialStore, Secret, SecretRequest};
use nocterm_workspace::SessionSpec;
use std::{rc::Rc, sync::Arc};

/// The credential is saved, but attaching its ID to connection history failed.
#[derive(Debug, thiserror::Error)]
pub enum CredentialAssociationError {
    #[error("could not associate the saved credential: {0}")]
    Binding(#[source] Arc<dyn std::error::Error + Send + Sync>),
}
impl CredentialAssociationError {
    pub fn binding(error: impl std::error::Error + Send + Sync + 'static) -> Self {
        Self::Binding(Arc::new(error))
    }
}

pub(crate) type Saved = Rc<
    dyn Fn(&SessionSpec, CredentialId, &mut App) -> Task<Result<(), CredentialAssociationError>>,
>;
pub(crate) struct ActiveCredentials {
    store: Arc<dyn CredentialStore>,
    saved: Saved,
}
impl Global for ActiveCredentials {}
impl ActiveCredentials {
    pub(crate) fn store(&self) -> &Arc<dyn CredentialStore> {
        &self.store
    }
    pub(crate) fn association(&self) -> Saved {
        self.saved.clone()
    }
}

/// Composition-root callback associates an encrypted credential ID with saved
/// connections/recent targets. It receives no plaintext secret.
pub fn init_credentials(
    store: Arc<dyn CredentialStore>,
    on_saved: impl Fn(
        &SessionSpec,
        CredentialId,
        &mut App,
    ) -> Task<Result<(), CredentialAssociationError>>
    + 'static,
    cx: &mut App,
) {
    cx.set_global(ActiveCredentials {
        store,
        saved: Rc::new(on_saved),
    });
}

#[derive(Default)]
pub(crate) struct CredentialState {
    epoch: u64,
    candidate: Option<(SecretRequest, Secret)>,
    lookup: Option<Task<()>>,
    message: Option<String>,
    /// The prompt has a saved secret and waits for the vault to unlock.
    awaiting_vault: bool,
}
impl CredentialState {
    pub(crate) fn clear(&mut self) {
        self.invalidate();
        self.candidate = None;
        self.awaiting_vault = false;
    }
    pub(crate) fn invalidate(&mut self) {
        self.epoch = self.epoch.wrapping_add(1);
        self.lookup = None;
    }
    pub(crate) fn begin_lookup(&mut self) {
        self.invalidate();
        self.awaiting_vault = false;
        self.message = None;
    }
    pub(crate) fn epoch(&self) -> u64 {
        self.epoch
    }
    pub(crate) fn matches(&self, epoch: u64) -> bool {
        self.epoch == epoch
    }
    pub(crate) fn message(&self) -> Option<&str> {
        self.message.as_deref()
    }
    pub(crate) fn set_message(&mut self, message: Option<String>) {
        self.message = message;
    }
    pub(crate) fn awaiting_vault(&self) -> bool {
        self.awaiting_vault
    }
    pub(crate) fn wait_for_vault(&mut self, waiting: bool) {
        self.awaiting_vault = waiting;
    }
    pub(crate) fn install_lookup(&mut self, task: Task<()>) {
        self.lookup = Some(task);
    }
    pub(crate) fn cancel_candidate(&mut self) {
        self.candidate = None;
    }
    pub(crate) fn remember_candidate(&mut self, request: SecretRequest, secret: Secret) {
        self.candidate = Some((request, secret));
    }
    pub(crate) fn take_candidate(&mut self) -> Option<(SecretRequest, Secret)> {
        self.candidate.take()
    }
}
/// What a secret prompt asks for, whether the last answer was wrong, and
/// whether the answer is hidden while typed.
pub(crate) fn describe(request: &SecretRequest) -> (String, bool, bool) {
    match request {
        SecretRequest::Password { target, retry } => {
            (format!("Password for {target}"), *retry, true)
        }
        SecretRequest::KeyPassphrase { path, retry } => {
            (format!("Passphrase for {}", path.display()), *retry, true)
        }
        SecretRequest::Interactive { prompt, echo } => {
            let prompt = prompt.trim();
            let title = if prompt.is_empty() {
                "The host asks for a response"
            } else {
                prompt
            };
            (title.to_owned(), false, !echo)
        }
    }
}

#[cfg(test)]
pub(crate) mod fake;
#[cfg(test)]
mod tests;
