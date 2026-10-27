//! Signing in to a server: address, certificate trust, then a password or the browser.
//!
//! One [`SignInSession`] is one run of the dialog. It owns the order of the steps and
//! publishes the server to the settings in a single edit once the sign-in worked.

use std::sync::Arc;

use futures::FutureExt as _;
use gpui::{Context, EventEmitter, Task};
use nocterm_hubs_model::{
    Account, Fingerprint, HubAddress, HubEndpointSpec, HubError, HubId, HubInfo, HubRecord,
    HubSignIn, HubTrust, HubsSettings, OidcProvider, OidcSignIn, ProbedHub, TrustDecision,
    TrustPolicy, decide,
};
use nocterm_session::Secret;
use nocterm_store::SettingsHandle;

use crate::error::SignInError;
use crate::installed::Deps;

/// How the certificate of the server came to be trusted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrustNote {
    /// It matches the fingerprint the user typed (or the record states).
    Pinned,
    /// A public certificate authority vouches for it.
    PublicCa,
    /// The user confirmed its fingerprint on first use.
    Confirmed,
}

/// Where the dialog is.
#[derive(Clone, Debug)]
pub enum SignInStep {
    /// Waiting for an address and an optional fingerprint.
    Address {
        problem: Option<SignInError>,
    },
    /// Reading the certificate or connecting.
    Checking,
    /// The certificate is not publicly trusted and no fingerprint was typed: the user
    /// decides whether to trust and pin it.
    ConfirmFingerprint {
        address: HubAddress,
        fingerprint: Fingerprint,
    },
    /// The certificate is not the one the user typed.
    Mismatch {
        typed: Fingerprint,
        presented: Fingerprint,
    },
    /// The certificate is trusted; waiting for a password or a provider.
    Method {
        hub_name: String,
        password: bool,
        providers: Vec<OidcProvider>,
        note: TrustNote,
        problem: Option<SignInError>,
    },
    /// A password was sent.
    Authenticating,
    /// Waiting for the user to finish in the browser.
    Browser {
        provider: String,
        /// The address to open; absent until the sign-in listens.
        url: Option<String>,
    },
    /// Adding the server to the settings.
    Saving,
    Finished(HubId),
    /// The server could not be added. Final.
    Failed(SignInError),
}

/// What the dialog asks its host to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SignInEvent {
    /// Open this address in the system browser.
    OpenBrowser(String),
    /// The server is signed in to and saved.
    Finished(HubId),
}

/// What is known of the server being signed in to.
#[derive(Default)]
struct Attempt {
    address: Option<HubAddress>,
    /// The fingerprint the user typed.
    expected: Option<Fingerprint>,
    trust: Option<TrustPolicy>,
    note: Option<TrustNote>,
    info: Option<HubInfo>,
    /// The label of a server that is already known.
    label: Option<String>,
    /// This run created the credentials, so a failed save has to erase them.
    fresh_credentials: bool,
    /// The address cannot be changed: signing in again to a known server.
    fixed: bool,
}

/// One run of the sign-in dialog.
pub struct SignInSession {
    trust: Arc<dyn HubTrust>,
    sign_in: Arc<dyn HubSignIn>,
    settings: SettingsHandle,
    step: SignInStep,
    attempt: Attempt,
    /// The call in flight; dropping it cancels a browser sign-in.
    work: Option<Task<()>>,
}

impl EventEmitter<SignInEvent> for SignInSession {}

impl SignInSession {
    pub(crate) fn new(deps: &Deps, settings: SettingsHandle) -> Self {
        Self {
            trust: Arc::clone(&deps.trust),
            sign_in: Arc::clone(&deps.sign_in),
            settings,
            step: SignInStep::Address { problem: None },
            attempt: Attempt::default(),
            work: None,
        }
    }

    /// Signing in again to a known server: its trust is decided, so the dialog goes straight
    /// to connecting.
    pub(crate) fn resume(
        deps: &Deps,
        settings: SettingsHandle,
        record: HubRecord,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut session = Self::new(deps, settings);
        let note = match record.trust {
            TrustPolicy::Pinned(_) => TrustNote::Pinned,
            TrustPolicy::PublicCa => TrustNote::PublicCa,
        };
        session.attempt = Attempt {
            address: Some(record.address.clone()),
            label: Some(record.label.clone()),
            fixed: true,
            ..Attempt::default()
        };
        session.connect(record.trust, note, cx);
        session
    }

    pub fn step(&self) -> &SignInStep {
        &self.step
    }

    /// The server may be edited: the dialog was not opened for a known one.
    pub fn address_editable(&self) -> bool {
        !self.attempt.fixed
    }

    fn move_to(&mut self, step: SignInStep, cx: &mut Context<Self>) {
        self.step = step;
        cx.notify();
    }

    /// Reads the certificate of the typed server. `fingerprint` may be empty.
    pub fn submit_address(&mut self, address: &str, fingerprint: &str, cx: &mut Context<Self>) {
        if !matches!(self.step, SignInStep::Address { .. }) || self.attempt.fixed {
            return;
        }
        let parsed = parse_inputs(address, fingerprint);
        let (address, expected) = match parsed {
            Ok(parsed) => parsed,
            Err(problem) => {
                return self.move_to(
                    SignInStep::Address {
                        problem: Some(problem),
                    },
                    cx,
                );
            }
        };
        // A known server keeps the certificate it was pinned with: a new sign-in must not
        // replace it, so a changed certificate is dealt with by removing the server first.
        let known = HubId::from(address.clone());
        if self
            .settings
            .setting::<HubsSettings>(cx)
            .find(&known)
            .is_some()
        {
            return self.back_to_address(SignInError::AlreadyKnown(known), cx);
        }
        self.attempt.address = Some(address.clone());
        self.attempt.expected = expected;
        self.move_to(SignInStep::Checking, cx);
        let probing = self.trust.probe(&address);
        self.work = Some(cx.spawn(async move |this, cx| {
            let probed = probing.await;
            this.update(cx, |session, cx| session.probed(probed, cx))
                .ok();
        }));
    }

    fn probed(&mut self, probed: Result<ProbedHub, HubError>, cx: &mut Context<Self>) {
        self.work = None;
        let probe = match probed {
            Ok(probe) => probe,
            Err(error) => return self.back_to_address(error.into(), cx),
        };
        let decision = decide(&probe, self.attempt.expected.as_ref());
        match decision {
            TrustDecision::Pinned(_) | TrustDecision::PublicCa => {
                let note = if matches!(decision, TrustDecision::PublicCa) {
                    TrustNote::PublicCa
                } else {
                    TrustNote::Pinned
                };
                if let Some(trust) = decision.settled() {
                    self.connect(trust, note, cx);
                }
            }
            TrustDecision::ConfirmFirstUse(fingerprint) => {
                if let Some(address) = self.attempt.address.clone() {
                    self.move_to(
                        SignInStep::ConfirmFingerprint {
                            address,
                            fingerprint,
                        },
                        cx,
                    );
                }
            }
            TrustDecision::Mismatch { typed, presented } => {
                self.move_to(SignInStep::Mismatch { typed, presented }, cx);
            }
        }
    }

    /// The user trusts the certificate that was shown and wants it pinned.
    pub fn confirm_fingerprint(&mut self, cx: &mut Context<Self>) {
        let SignInStep::ConfirmFingerprint { fingerprint, .. } = self.step else {
            return;
        };
        self.connect(TrustPolicy::Pinned(fingerprint), TrustNote::Confirmed, cx);
    }

    fn back_to_address(&mut self, problem: SignInError, cx: &mut Context<Self>) {
        self.move_to(
            SignInStep::Address {
                problem: Some(problem),
            },
            cx,
        );
    }

    /// Connects with the decided trust and learns which ways of signing in the server offers.
    fn connect(&mut self, trust: TrustPolicy, note: TrustNote, cx: &mut Context<Self>) {
        let Some(address) = self.attempt.address.clone() else {
            return;
        };
        self.attempt.trust = Some(trust);
        self.attempt.note = Some(note);
        self.move_to(SignInStep::Checking, cx);
        let connecting = self.sign_in.connect(HubEndpointSpec { address, trust });
        self.work = Some(cx.spawn(async move |this, cx| {
            let result = connecting.await;
            this.update(cx, |session, cx| session.connected(result, cx))
                .ok();
        }));
    }

    fn connected(&mut self, result: Result<HubInfo, HubError>, cx: &mut Context<Self>) {
        self.work = None;
        let info = match result {
            Ok(info) => info,
            Err(error) => return self.back_to_address(error.into(), cx),
        };
        if !info.bootstrapped {
            return self.back_to_address(SignInError::NotInitialized, cx);
        }
        let signed_in = info.signed_in;
        self.attempt.info = Some(info);
        if signed_in {
            self.publish(cx);
        } else {
            self.show_methods(None, cx);
        }
    }

    fn show_methods(&mut self, problem: Option<SignInError>, cx: &mut Context<Self>) {
        let Some(info) = &self.attempt.info else {
            return;
        };
        let step = SignInStep::Method {
            hub_name: info.name.clone(),
            password: info.password_login,
            providers: info.oidc_providers.clone(),
            note: self.attempt.note.unwrap_or(TrustNote::Pinned),
            problem,
        };
        self.move_to(step, cx);
    }

    fn endpoint(&self) -> Option<HubEndpointSpec> {
        Some(HubEndpointSpec {
            address: self.attempt.address.clone()?,
            trust: self.attempt.trust?,
        })
    }

    /// Signs in with a user name and password.
    pub fn sign_in_with_password(
        &mut self,
        username: &str,
        password: Secret,
        cx: &mut Context<Self>,
    ) {
        let (SignInStep::Method { .. }, Some(endpoint)) = (&self.step, self.endpoint()) else {
            return;
        };
        self.move_to(SignInStep::Authenticating, cx);
        let signing_in = self
            .sign_in
            .password(endpoint, username.trim().to_owned(), password);
        self.work = Some(cx.spawn(async move |this, cx| {
            let result = signing_in.await;
            this.update(cx, |session, cx| session.signed(result, cx))
                .ok();
        }));
    }

    /// Signs in through the identity provider `provider` in the system browser.
    pub fn sign_in_with_provider(&mut self, provider: &str, cx: &mut Context<Self>) {
        let (SignInStep::Method { .. }, Some(endpoint)) = (&self.step, self.endpoint()) else {
            return;
        };
        self.move_to(
            SignInStep::Browser {
                provider: provider.to_owned(),
                url: None,
            },
            cx,
        );
        let OidcSignIn { browser, done } = self.sign_in.oidc(endpoint, provider.to_owned());
        self.work = Some(cx.spawn(async move |this, cx| {
            let result = await_browser_sign_in(browser, done, |url| {
                this.update(cx, |session, cx| session.browser_ready(url, cx))
                    .ok();
            })
            .await;
            this.update(cx, |session, cx| session.signed(result, cx))
                .ok();
        }));
    }

    fn browser_ready(&mut self, url: String, cx: &mut Context<Self>) {
        if let SignInStep::Browser { url: shown, .. } = &mut self.step {
            *shown = Some(url.clone());
            cx.emit(SignInEvent::OpenBrowser(url));
            cx.notify();
        }
    }

    fn signed(&mut self, result: Result<Account, HubError>, cx: &mut Context<Self>) {
        self.work = None;
        match result {
            Ok(_) => {
                self.attempt.fresh_credentials = true;
                self.publish(cx);
            }
            Err(error) => self.show_methods(Some(error.into()), cx),
        }
    }

    /// Goes one step back; a browser sign-in in flight is cancelled.
    pub fn back(&mut self, cx: &mut Context<Self>) {
        let target = match &self.step {
            SignInStep::Browser { .. } | SignInStep::Authenticating => {
                self.work = None;
                self.show_methods(None, cx);
                return;
            }
            SignInStep::Method { .. } if self.attempt.fixed => return,
            SignInStep::Checking => {
                self.work = None;
                SignInStep::Address { problem: None }
            }
            SignInStep::ConfirmFingerprint { .. }
            | SignInStep::Mismatch { .. }
            | SignInStep::Method { .. } => SignInStep::Address { problem: None },
            SignInStep::Address { .. }
            | SignInStep::Saving
            | SignInStep::Finished(_)
            | SignInStep::Failed(_) => return,
        };
        self.move_to(target, cx);
    }

    /// Adds the server to the settings in one edit.
    fn publish(&mut self, cx: &mut Context<Self>) {
        let (Some(address), Some(trust), Some(info)) = (
            self.attempt.address.clone(),
            self.attempt.trust,
            self.attempt.info.as_ref(),
        ) else {
            return;
        };
        let label = self
            .attempt
            .label
            .clone()
            .unwrap_or_else(|| info.name.clone());
        let record = HubRecord::new(address, trust, label);
        let hub = record.id();
        self.move_to(SignInStep::Saving, cx);
        let saving = self
            .settings
            .update_checked::<HubsSettings>(cx, move |settings| {
                // Signing in again to a known server keeps the colour its owner chose.
                let color = settings.find(&record.id()).and_then(|known| known.color);
                settings.upsert(record.with_color(color));
                Ok(())
            });
        self.work = Some(cx.spawn(async move |this, cx| {
            let saved = saving.await;
            this.update(cx, |session, cx| session.saved(&hub, saved.map(|_| ()), cx))
                .ok();
        }));
    }

    fn saved(
        &mut self,
        hub: &HubId,
        saved: Result<(), nocterm_store::StoreError>,
        cx: &mut Context<Self>,
    ) {
        self.work = None;
        match saved {
            Ok(()) => {
                self.move_to(SignInStep::Finished(hub.clone()), cx);
                cx.emit(SignInEvent::Finished(hub.clone()));
            }
            Err(error) => {
                if self.attempt.fresh_credentials {
                    // The credentials were made for a server that never got listed.
                    if let Err(forget) = self.sign_in.forget(hub) {
                        tracing::warn!(%hub, error = %forget, "credentials of an unlisted hub were kept");
                    }
                }
                self.move_to(SignInStep::Failed(SignInError::Save(error)), cx);
            }
        }
    }
}

fn parse_inputs(
    address: &str,
    fingerprint: &str,
) -> Result<(HubAddress, Option<Fingerprint>), SignInError> {
    let address = HubAddress::parse(address)?;
    let fingerprint = fingerprint.trim();
    let expected = if fingerprint.is_empty() {
        None
    } else {
        Some(Fingerprint::parse(fingerprint)?)
    };
    Ok((address, expected))
}

/// Waits for a browser sign-in, reporting the address to open as soon as it is known.
async fn await_browser_sign_in(
    browser: futures::channel::oneshot::Receiver<String>,
    done: nocterm_hubs_model::HubFuture<Account>,
    mut open: impl FnMut(String),
) -> Result<Account, HubError> {
    let mut browser = browser.fuse();
    let mut done = done.fuse();
    loop {
        futures::select! {
            url = browser => {
                if let Ok(url) = url {
                    open(url);
                }
            }
            result = done => return result,
        }
    }
}

#[cfg(test)]
mod tests;
