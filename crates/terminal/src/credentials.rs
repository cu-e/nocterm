//! Prompts answered from, and remembered in, a [`CredentialStore`].
use crate::{Terminal, TerminalEvent};
use gpui_kit::{App, Context, Global, Task};
use nocterm_session::{CredentialId, CredentialStore, Prompt, Secret, SecretRequest};
use nocterm_workspace::SessionSpec;
use std::{rc::Rc, sync::Arc};

type Saved = Rc<dyn Fn(&SessionSpec, CredentialId, &mut App) -> Task<Result<(), String>>>;
pub(crate) struct ActiveCredentials {
    store: Arc<dyn CredentialStore>,
    saved: Saved,
}
impl Global for ActiveCredentials {}

/// Composition-root callback associates an encrypted credential ID with saved
/// connections/recent targets. It receives no plaintext secret.
pub fn init_credentials(
    store: Arc<dyn CredentialStore>,
    on_saved: impl Fn(&SessionSpec, CredentialId, &mut App) -> Task<Result<(), String>> + 'static,
    cx: &mut App,
) {
    cx.set_global(ActiveCredentials {
        store,
        saved: Rc::new(on_saved),
    });
}

#[derive(Default)]
pub(crate) struct CredentialState {
    pub epoch: u64,
    candidate: Option<(SecretRequest, Secret)>,
    lookup: Option<Task<()>>,
    pub message: Option<String>,
    /// The prompt has a saved secret and waits for the vault to unlock.
    awaiting_vault: bool,
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
impl Terminal {
    pub(crate) fn clear_credentials(&mut self) {
        self.credentials.epoch = self.credentials.epoch.wrapping_add(1);
        self.credentials.candidate = None;
        self.credentials.lookup = None;
        self.credentials.awaiting_vault = false;
    }
    pub fn vault_unlocked(&self, cx: &App) -> bool {
        cx.try_global::<ActiveCredentials>()
            .is_some_and(|provider| provider.store.is_available())
    }
    /// Whether unlocking the vault would answer the current prompt.
    pub fn awaiting_vault(&self) -> bool {
        self.credentials.awaiting_vault && self.prompt().is_some()
    }
    pub fn credential_message(&self) -> Option<&str> {
        self.credentials.message.as_deref()
    }
    pub fn clear_credential_message(&mut self, cx: &mut Context<Self>) {
        self.credentials.message = None;
        cx.emit(TerminalEvent::Changed);
    }
    pub(crate) fn retrieve_credential(&mut self, cx: &mut Context<Self>) {
        self.credentials.epoch = self.credentials.epoch.wrapping_add(1);
        self.credentials.lookup = None;
        self.credentials.awaiting_vault = false;
        self.credentials.message = None;
        let Some(Prompt::Secret { request, .. }) = self.prompt() else {
            return;
        };
        let request = request.clone();
        if !matches!(request, SecretRequest::Interactive { .. }) {
            self.credentials.candidate = None;
        }
        if matches!(
            request,
            SecretRequest::Password { retry: true, .. }
                | SecretRequest::KeyPassphrase { retry: true, .. }
        ) {
            return;
        }
        if !request.is_storable() {
            return;
        }
        let Some(id) = self.spec().credential else {
            return;
        };
        let Some(provider) = cx.try_global::<ActiveCredentials>() else {
            return;
        };
        if !provider.store.is_available() {
            self.credentials.message = Some(
                "Unlock the credential vault to use the saved secret, or enter it here.".into(),
            );
            self.credentials.awaiting_vault = true;
            let available = provider.store.wait_available();
            self.wait_for_unlock(available, cx);
            return;
        }
        let future = provider.store.get(id, &request);
        let epoch = self.credentials.epoch;
        self.credentials.lookup = Some(cx.spawn(async move |this, cx| {
            let result = future.await;
            let _ = this.update(cx, |this, cx| {
                if epoch != this.credentials.epoch || this.prompt().is_none() {
                    return;
                }
                match result {
                    Ok(secret) => this.answer_secret(Some(secret), cx),
                    Err(error) => {
                        this.credentials.message =
                            Some(format!("Saved credential unavailable: {error}"));
                        cx.emit(TerminalEvent::Changed);
                    }
                }
            });
        }));
    }
    /// Answers the prompt with the saved secret as soon as the vault is
    /// unlocked, whether from the prompt's link, Settings or device unlock.
    fn wait_for_unlock(
        &mut self,
        available: impl Future<Output = ()> + 'static,
        cx: &mut Context<Self>,
    ) {
        let epoch = self.credentials.epoch;
        self.credentials.lookup = Some(cx.spawn(async move |this, cx| {
            available.await;
            let _ = this.update(cx, |this, cx| {
                if epoch == this.credentials.epoch && this.prompt().is_some() {
                    this.credentials.awaiting_vault = false;
                    this.retrieve_credential(cx);
                    cx.emit(TerminalEvent::Changed);
                }
            });
        }));
    }
    /// Remember is an explicit user choice and never applies to interactive/MFA.
    pub fn answer_secret_and_remember(
        &mut self,
        secret: Secret,
        remember: bool,
        cx: &mut Context<Self>,
    ) {
        self.credentials.epoch = self.credentials.epoch.wrapping_add(1);
        self.credentials.lookup = None;
        let interactive = matches!(
            self.prompt(),
            Some(Prompt::Secret {
                request: SecretRequest::Interactive { .. },
                ..
            })
        );
        if !interactive {
            self.credentials.candidate = None;
        }
        if remember
            && self.vault_unlocked(cx)
            && let Some(Prompt::Secret { request, .. }) = self.prompt()
            && request.is_storable()
        {
            self.credentials.candidate = Some((request.clone(), secret.clone()));
        }
        self.answer_secret(Some(secret), cx);
    }
    pub(crate) fn cancel_credential_candidate(&mut self) {
        self.credentials.candidate = None;
    }
    pub(crate) fn save_authenticated_credential(&mut self, cx: &mut Context<Self>) {
        let Some((request, secret)) = self.credentials.candidate.take() else {
            return;
        };
        let Some(provider) = cx.try_global::<ActiveCredentials>() else {
            return;
        };
        let saved = provider.saved.clone();
        let spec = self.spec().clone();
        // Association is published only after the credential is saved.
        let future = provider
            .store
            .save(spec.title.to_string(), &request, secret);
        let epoch = self.credentials.epoch;
        cx.spawn(async move |this, cx| {
            let result = future.await;
            let association = this.update(cx, |this, cx| {
                if epoch != this.credentials.epoch {
                    return None;
                }
                match result {
                    Ok(id) => {
                        this.set_credential_id(id);
                        return Some(saved(&spec, id, cx));
                    }
                    Err(error) => {
                        this.credentials.message =
                            Some(format!("Could not save credential: {error}"))
                    }
                }
                cx.emit(TerminalEvent::Changed);
                None
            });
            if let Ok(Some(association)) = association {
                let result = association.await;
                let _ = this.update(cx, |this, cx| {
                    if epoch != this.credentials.epoch {
                        return;
                    }
                    this.credentials.message = Some(match result {
                        Ok(()) => "Credential saved in the encrypted vault.".into(),
                        Err(error) => format!(
                            "Credential saved; could not link it to connection history: {error}"
                        ),
                    });
                    cx.emit(TerminalEvent::Changed);
                });
            }
        })
        .detach();
    }
}

#[cfg(test)]
pub(crate) mod fake;
#[cfg(test)]
mod tests;
