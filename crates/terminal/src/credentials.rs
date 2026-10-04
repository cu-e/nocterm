//! Authentication integration depends on the vault domain, never its UI feature.
use crate::{Terminal, TerminalEvent};
use gpui_kit::{App, Context, Global, Task};
use nocterm_session::{CredentialId, Prompt, Secret, SecretRequest};
use nocterm_vault::{CredentialBinding, VaultService};
use nocterm_workspace::SessionSpec;
use std::{rc::Rc, sync::Arc, time::Duration};

/// How often a prompt waiting for the vault checks whether it was unlocked.
const UNLOCK_POLL: Duration = Duration::from_millis(250);

type Saved = Rc<dyn Fn(&SessionSpec, CredentialId, &mut App) -> Task<Result<(), String>>>;
pub(crate) struct ActiveCredentials {
    service: Arc<VaultService>,
    saved: Saved,
}
impl Global for ActiveCredentials {}

/// Composition-root callback associates an encrypted credential ID with saved
/// connections/recent targets. It receives no plaintext secret.
pub fn init_credentials(
    service: Arc<VaultService>,
    on_saved: impl Fn(&SessionSpec, CredentialId, &mut App) -> Task<Result<(), String>> + 'static,
    cx: &mut App,
) {
    cx.set_global(ActiveCredentials {
        service,
        saved: Rc::new(on_saved),
    });
}

#[derive(Default)]
pub(crate) struct CredentialState {
    pub epoch: u64,
    candidate: Option<(CredentialBinding, Secret)>,
    lookup: Option<Task<()>>,
    pub message: Option<String>,
    /// The prompt has a saved secret and waits for the vault to unlock.
    awaiting_vault: bool,
}
fn binding(request: &SecretRequest) -> Option<CredentialBinding> {
    match request {
        SecretRequest::Password { target, .. } => Some(CredentialBinding::Password {
            target: target.clone(),
        }),
        SecretRequest::KeyPassphrase { path, .. } => {
            Some(CredentialBinding::KeyPassphrase { path: path.clone() })
        }
        SecretRequest::Interactive { .. } => None,
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
impl Terminal {
    pub(crate) fn clear_credentials(&mut self) {
        self.credentials.epoch = self.credentials.epoch.wrapping_add(1);
        self.credentials.candidate = None;
        self.credentials.lookup = None;
        self.credentials.awaiting_vault = false;
    }
    pub fn vault_unlocked(&self, cx: &App) -> bool {
        cx.try_global::<ActiveCredentials>()
            .is_some_and(|provider| provider.service.is_unlocked())
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
        let Some(binding) = binding(&request) else {
            return;
        };
        let Some(id) = self.spec().credential else {
            return;
        };
        let Some(provider) = cx.try_global::<ActiveCredentials>() else {
            return;
        };
        if !provider.service.is_unlocked() {
            self.credentials.message = Some(
                "Unlock the credential vault to use the saved secret, or enter it here.".into(),
            );
            self.credentials.awaiting_vault = true;
            let service = provider.service.clone();
            self.wait_for_unlock(service, cx);
            return;
        }
        let future = provider.service.get(id, binding);
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
    fn wait_for_unlock(&mut self, service: Arc<VaultService>, cx: &mut Context<Self>) {
        let epoch = self.credentials.epoch;
        self.credentials.lookup = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(UNLOCK_POLL).await;
                if !service.is_unlocked() {
                    continue;
                }
                let _ = this.update(cx, |this, cx| {
                    if epoch == this.credentials.epoch && this.prompt().is_some() {
                        this.credentials.awaiting_vault = false;
                        this.retrieve_credential(cx);
                        cx.emit(TerminalEvent::Changed);
                    }
                });
                return;
            }
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
            && let Some(binding) = binding(request)
        {
            self.credentials.candidate = Some((binding, secret.clone()));
        }
        self.answer_secret(Some(secret), cx);
    }
    pub(crate) fn cancel_credential_candidate(&mut self) {
        self.credentials.candidate = None;
    }
    pub(crate) fn save_authenticated_credential(&mut self, cx: &mut Context<Self>) {
        let Some((binding, secret)) = self.credentials.candidate.take() else {
            return;
        };
        let Some(provider) = cx.try_global::<ActiveCredentials>() else {
            return;
        };
        let service = provider.service.clone();
        let saved = provider.saved.clone();
        let spec = self.spec().clone();
        // A new ID avoids replacing a different binding referenced by an edited
        // profile. Association is published only after the ciphertext commits.
        let future = service.put(None, spec.title.to_string(), binding, secret);
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
mod tests;
