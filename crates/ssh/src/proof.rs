//! Purpose-bound initial login; ordinary desktop authentication is a separate path.
mod device;
mod observer;
use crate::{
    login::{self, Client, Observed},
    proxy,
    source_stream::SourceStream,
};
pub use device::{DeviceLoginAuthorizer, LoginConsentRequest};
use nocterm_cloud_model::login::{
    LoginConsentBinding, LoginError, LoginProofRequest, SessionLoginAuthorizer,
};
use nocterm_session::{
    ConnectRequest, ConnectionFailure, ConnectionFailureKind, ProxyConfig, SessionDriver,
    SessionError, Target,
};
use nocterm_ssh_keys::{PublicKeyText, login::UserauthLogin};
use observer::{Observation, ObservedStream};
use russh::{
    client::Handle,
    keys::{HashAlg, PublicKey, agent::AgentIdentity},
};
use std::sync::{Arc, Mutex};
/// Run-specific login view. Holds public facts and a constrained remote capability,
/// never a private key or an ambient password/agent/file authentication option.
pub struct SessionLoginAttempt {
    binding: LoginConsentBinding,
    target: Target,
    pinned_host: PublicKeyText,
    authorizer: Arc<dyn SessionLoginAuthorizer>,
}
impl SessionLoginAttempt {
    pub fn new(
        binding: LoginConsentBinding,
        target: Target,
        pinned_host: PublicKeyText,
        authorizer: Arc<dyn SessionLoginAuthorizer>,
    ) -> Result<Self, LoginError> {
        nocterm_ssh_keys::login::login_key_blob(&pinned_host)?;
        if target.user != binding.target().principal().as_str()
            || binding.target().host().as_bytes()
                != &pinned_host
                    .fingerprint_bytes()
                    .map_err(|_| LoginError::Mismatch)?
            || binding.authentication().key().as_bytes()
                != &binding
                    .user_key()
                    .fingerprint_bytes()
                    .map_err(|_| LoginError::Mismatch)?
        {
            return Err(LoginError::Mismatch);
        }
        Ok(Self {
            binding,
            target,
            pinned_host,
            authorizer,
        })
    }
    pub(crate) fn check(&self, request: &ConnectRequest) -> Result<(), SessionError> {
        if request.proxy != ProxyConfig::Direct || !request.jumps.is_empty() {
            return Err(login_error(LoginError::Unsupported));
        }
        if request.target != self.target {
            return Err(login_error(LoginError::Mismatch));
        }
        Ok(())
    }
}
pub(crate) async fn establish(
    attempt: SessionLoginAttempt,
    request: &ConnectRequest,
    driver: &SessionDriver,
    observed: &Arc<Mutex<Observed>>,
) -> Result<Handle<Client>, SessionError> {
    attempt.check(request)?;
    let seen = Arc::new(Mutex::new(Observation::default()));
    let stream = proxy::connect(&request.target, &ProxyConfig::Direct, None)
        .await
        .map_err(|error| error.into_session(&request.target.host))?;
    let stream = SourceStream::new(stream, driver.clone());
    let stream = ObservedStream::new(stream, seen.clone());
    let pinned = PublicKey::from_openssh(attempt.pinned_host.as_str()).map_err(native_error)?;
    let mut handle = login::handshake_pinned(
        stream,
        &request.target,
        pinned,
        request.keepalive_interval,
        observed,
    )
    .await?;
    let public =
        PublicKey::from_openssh(attempt.binding.user_key().as_str()).map_err(native_error)?;
    let mut signer = LoginSigner { attempt, seen };
    let result = handle
        .authenticate_publickey_with(request.target.user.clone(), public, None, &mut signer)
        .await
        .map_err(|error| match error {
            SignerError::Authorization(error) => login_error(error),
            other => native_error(other),
        })?;
    if !result.success() {
        return Err(login_error(LoginError::NeedsUnlock));
    }
    // Dropping the signer also releases the remote/device authorizer. Rekey and
    // the authenticated connection require no further device signatures.
    drop(signer);
    Ok(handle)
}
struct LoginSigner {
    attempt: SessionLoginAttempt,
    seen: Arc<Mutex<Observation>>,
}
#[derive(Debug, thiserror::Error)]
enum SignerError {
    #[error(transparent)]
    Send(#[from] russh::SendError),
    #[error(transparent)]
    Authorization(#[from] LoginError),
}
impl russh::Signer for LoginSigner {
    type Error = SignerError;
    async fn auth_sign(
        &mut self,
        _: &AgentIdentity,
        hash: Option<HashAlg>,
        mut bytes: Vec<u8>,
    ) -> Result<Vec<u8>, SignerError> {
        if hash.is_some() {
            return Err(LoginError::Unsupported.into());
        }
        let login = UserauthLogin::parse(bytes.clone()).map_err(LoginError::from)?;
        let host = self
            .seen
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .proof()
            .map_err(LoginError::from)?;
        let request = LoginProofRequest::new(self.attempt.binding.clone(), host, login.clone());
        request.verify(&self.attempt.pinned_host)?;
        let signature = self.attempt.authorizer.authorize_login(request).await?;
        signature.verify(&login).map_err(LoginError::from)?;
        let blob = signature.as_blob();
        bytes.extend((blob.len() as u32).to_be_bytes());
        bytes.extend(blob);
        Ok(bytes)
    }
}
pub(crate) fn native_error(error: impl std::error::Error + Send + Sync + 'static) -> SessionError {
    ConnectionFailure::new(ConnectionFailureKind::Backend, error).into()
}
fn login_error(error: LoginError) -> SessionError {
    let kind = match &error {
        LoginError::NeedsUnlock
        | LoginError::Replay
        | LoginError::Credential(_)
        | LoginError::Expired
        | LoginError::ClockRollback => ConnectionFailureKind::NeedsUnlock,
        LoginError::Unsupported => ConnectionFailureKind::Unsupported,
        LoginError::Backend(_) => ConnectionFailureKind::Backend,
        _ => ConnectionFailureKind::Denied,
    };
    ConnectionFailure::new(kind, error).into()
}
