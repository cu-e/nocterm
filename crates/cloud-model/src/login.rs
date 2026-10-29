//! Finite device-issued, single-purpose SSH consent. IDs are descriptive; the
//! injected authority checks actual captured account/run/credential membership.
use crate::{
    authority::{AuthenticationBinding, RunBinding, TargetBinding},
    identity::{AttemptId, Timestamp},
};
use futures::future::BoxFuture;
use nocterm_core::SharedCause;
use nocterm_ssh_keys::{
    PublicKeyText,
    login::{InitialHostProof, LoginProofError, LoginSignature, UserauthLogin},
};
use std::collections::BTreeMap;

pub const MAX_LOGIN_CONSENTS: usize = 32;
pub const MAX_LOGIN_CONSENT_MS: u64 = 60_000;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LoginConsentWindow {
    issued: Timestamp,
    expires: Timestamp,
}
impl LoginConsentWindow {
    pub fn new(issued: Timestamp, expires: Timestamp) -> Result<Self, LoginError> {
        if expires
            .elapsed_since(issued)
            .is_none_or(|ms| ms == 0 || ms > MAX_LOGIN_CONSENT_MS)
        {
            return Err(LoginError::InvalidConsent);
        }
        Ok(Self { issued, expires })
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoginConsentBinding {
    run: RunBinding,
    target: TargetBinding,
    authentication: AuthenticationBinding,
    attempt: AttemptId,
    nonce: [u8; 32],
    issued: Timestamp,
    expires: Timestamp,
    user_key: PublicKeyText,
}
impl LoginConsentBinding {
    pub fn new(
        run: RunBinding,
        target: TargetBinding,
        authentication: AuthenticationBinding,
        attempt: AttemptId,
        nonce: [u8; 32],
        window: LoginConsentWindow,
        user_key: PublicKeyText,
    ) -> Result<Self, LoginError> {
        nocterm_ssh_keys::login::login_key_blob(&user_key)?;
        let LoginConsentWindow { issued, expires } = window;
        if nonce == [0; 32] {
            return Err(LoginError::InvalidConsent);
        }
        Ok(Self {
            run,
            target,
            authentication,
            attempt,
            nonce,
            issued,
            expires,
            user_key,
        })
    }
    pub fn run(&self) -> &RunBinding {
        &self.run
    }
    pub fn target(&self) -> &TargetBinding {
        &self.target
    }
    pub fn authentication(&self) -> &AuthenticationBinding {
        &self.authentication
    }
    pub fn attempt(&self) -> AttemptId {
        self.attempt
    }
    pub fn nonce(&self) -> &[u8; 32] {
        &self.nonce
    }
    pub fn issued(&self) -> Timestamp {
        self.issued
    }
    pub fn expires(&self) -> Timestamp {
        self.expires
    }
    pub fn user_key(&self) -> &PublicKeyText {
        &self.user_key
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoginProofRequest {
    binding: LoginConsentBinding,
    host: InitialHostProof,
    login: UserauthLogin,
}
impl LoginProofRequest {
    pub fn new(binding: LoginConsentBinding, host: InitialHostProof, login: UserauthLogin) -> Self {
        Self {
            binding,
            host,
            login,
        }
    }
    pub fn binding(&self) -> &LoginConsentBinding {
        &self.binding
    }
    pub fn host(&self) -> &InitialHostProof {
        &self.host
    }
    pub fn login(&self) -> &UserauthLogin {
        &self.login
    }
    pub fn verify(&self, pinned_host: &PublicKeyText) -> Result<(), LoginError> {
        if self.login.principal() != self.binding.target.principal().as_str()
            || self.login.user_key() != self.binding.user_key()
        {
            return Err(LoginError::Mismatch);
        }
        self.host.verify(pinned_host, self.login.session_id())?;
        Ok(())
    }
}
/// The sole device signing port. Its input is a checked SSH login action, never
/// arbitrary bytes. Implementers must consume original finite consent first.
pub trait SessionLoginAuthorizer: Send + Sync + 'static {
    fn authorize_login(
        &self,
        request: LoginProofRequest,
    ) -> BoxFuture<'static, Result<LoginSignature, LoginError>>;
}
/// Nonblocking actual authority check, composed by the host at consent issuance.
/// It must check the original account, run/epochs, target, selected credential
/// revision and grant; a matching descriptive binding alone is insufficient.
pub trait LoginConsentAuthority: Send + Sync + 'static {
    fn check(&self, binding: &LoginConsentBinding) -> Result<(), LoginError>;
}
struct Consent {
    binding: LoginConsentBinding,
    host: PublicKeyText,
    used: bool,
}
/// Device-only in-memory owner. Recreating it loses consent and requires unlock;
/// neither a runner request nor a persisted receipt can re-create an entry.
pub struct LoginConsentLedger {
    entries: BTreeMap<AttemptId, Consent>,
    observed: Timestamp,
}
impl LoginConsentLedger {
    pub fn new(now: Timestamp) -> Self {
        Self {
            entries: BTreeMap::new(),
            observed: now,
        }
    }
    /// The device supplies random attempt/nonce and a user-approved exact binding.
    /// Entries are never evicted while live; expired tombstones may be pruned.
    pub fn issue(
        &mut self,
        binding: LoginConsentBinding,
        host: PublicKeyText,
        now: Timestamp,
    ) -> Result<(), LoginError> {
        self.time(now)?;
        nocterm_ssh_keys::login::login_key_blob(&host)?;
        if binding.target.host().as_bytes()
            != &host.fingerprint_bytes().map_err(|_| LoginError::Mismatch)?
            || binding.authentication.key().as_bytes()
                != &binding
                    .user_key
                    .fingerprint_bytes()
                    .map_err(|_| LoginError::Mismatch)?
        {
            return Err(LoginError::Mismatch);
        }
        if now < binding.issued || now >= binding.expires {
            return Err(LoginError::Expired);
        }
        if self.entries.contains_key(&binding.attempt) {
            return Err(LoginError::Replay);
        }
        self.entries.retain(|_, entry| entry.binding.expires > now);
        if self.entries.len() >= MAX_LOGIN_CONSENTS {
            return Err(LoginError::Capacity);
        }
        self.entries.insert(
            binding.attempt,
            Consent {
                binding,
                host,
                used: false,
            },
        );
        Ok(())
    }
    /// Consume atomically before any key loading or awaited operation. Even a
    /// later signing failure requires fresh user consent, never automatic retry.
    pub fn consume(
        &mut self,
        request: &LoginProofRequest,
        now: Timestamp,
    ) -> Result<(), LoginError> {
        self.time(now)?;
        let entry = self
            .entries
            .get_mut(&request.binding.attempt)
            .ok_or(LoginError::NeedsUnlock)?;
        if entry.binding != request.binding {
            return Err(LoginError::Mismatch);
        }
        if now >= entry.binding.expires {
            return Err(LoginError::Expired);
        }
        if entry.used {
            return Err(LoginError::Replay);
        }
        request.verify(&entry.host)?;
        entry.used = true;
        Ok(())
    }
    /// Recheck the original consumed attempt after awaited device work. This is
    /// neither a new consumption nor permission to re-execute another request.
    pub fn check_consumed(
        &mut self,
        request: &LoginProofRequest,
        now: Timestamp,
    ) -> Result<(), LoginError> {
        self.time(now)?;
        let entry = self
            .entries
            .get(&request.binding.attempt)
            .ok_or(LoginError::NeedsUnlock)?;
        if entry.binding != request.binding {
            return Err(LoginError::Mismatch);
        }
        if now >= entry.binding.expires {
            return Err(LoginError::Expired);
        }
        if !entry.used {
            return Err(LoginError::NeedsUnlock);
        }
        Ok(())
    }
    fn time(&mut self, now: Timestamp) -> Result<(), LoginError> {
        if now < self.observed {
            self.entries.clear();
            return Err(LoginError::ClockRollback);
        }
        self.observed = now;
        Ok(())
    }
}
#[derive(Clone, Debug, thiserror::Error)]
pub enum LoginError {
    #[error("fresh device consent and unlock are required")]
    NeedsUnlock,
    #[error("the SSH login consent is malformed")]
    InvalidConsent,
    #[error("the SSH login differs from the original consent")]
    Mismatch,
    #[error("the SSH login consent has expired")]
    Expired,
    #[error("the SSH login consent has already been used")]
    Replay,
    #[error("the finite SSH consent ledger is full")]
    Capacity,
    #[error("the login clock moved backwards; fresh consent is required")]
    ClockRollback,
    #[error("the original source or login authority is no longer valid")]
    Revoked,
    #[error("this SSH login route or algorithm is unsupported")]
    Unsupported,
    #[error(transparent)]
    Proof(#[from] LoginProofError),
    #[error("the selected device credential is unavailable: {0}")]
    Credential(#[source] SharedCause),
    #[error("SSH device authorization failed: {0}")]
    Backend(#[source] SharedCause),
}
impl LoginError {
    pub fn backend(error: impl std::error::Error + Send + Sync + 'static) -> Self {
        Self::Backend(SharedCause::new(error))
    }
}
