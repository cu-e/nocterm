//! Purpose-bound certificate login: a chat's in-memory Ed25519 key with the OpenSSH user
//! certificate a person's CA issued for it (ADR 0137).
//!
//! Like proof login it connects directly, pins the host key and never falls back to the
//! agent, key files, a vault prompt or a password. The key lives only in this value; it
//! is never written anywhere and is dropped with the connection's login step.
use crate::{
    login::{self, Client, Observed},
    proxy,
    source_stream::SourceStream,
};
use nocterm_session::{
    ConnectRequest, ConnectionFailure, ConnectionFailureKind, ProxyConfig, SessionDriver,
    SessionError, Target,
};
use nocterm_ssh_keys::PublicKeyText;
use russh::{
    client::Handle,
    keys::{
        Certificate, PrivateKey, PublicKey,
        ssh_key::{
            certificate::CertType,
            private::{Ed25519Keypair, KeypairData},
        },
    },
};
use std::sync::{Arc, Mutex};
use zeroize::Zeroizing;

/// Why a certificate login cannot be prepared.
#[derive(Debug, thiserror::Error)]
pub enum CertificateLoginError {
    #[error("the certificate cannot be read")]
    Certificate(#[source] russh::keys::ssh_key::Error),
    #[error("the certificate is not a user certificate for this key")]
    OtherKey,
    #[error("the pinned host key cannot be read")]
    HostKey(#[source] russh::keys::ssh_key::Error),
    #[error("the login key cannot be made")]
    Key(#[source] russh::keys::ssh_key::Error),
}

/// One login of `target` with a certificate for the key of `seed`, against a pinned host.
pub struct CertificateLogin {
    target: Target,
    pinned_host: PublicKey,
    key: Arc<PrivateKey>,
    certificate: Certificate,
}
impl CertificateLogin {
    pub fn new(
        target: Target,
        pinned_host: &PublicKeyText,
        seed: &[u8; 32],
        certificate: &[u8],
    ) -> Result<Self, CertificateLoginError> {
        let seed = Zeroizing::new(*seed);
        let key = PrivateKey::new(KeypairData::Ed25519(Ed25519Keypair::from_seed(&seed)), "")
            .map_err(CertificateLoginError::Key)?;
        let certificate =
            Certificate::from_bytes(certificate).map_err(CertificateLoginError::Certificate)?;
        if certificate.cert_type() != CertType::User
            || certificate.public_key() != key.public_key().key_data()
        {
            return Err(CertificateLoginError::OtherKey);
        }
        let pinned_host = PublicKey::from_openssh(pinned_host.as_str())
            .map_err(CertificateLoginError::HostKey)?;
        Ok(Self {
            target,
            pinned_host,
            key: Arc::new(key),
            certificate,
        })
    }
    pub(crate) fn check(&self, request: &ConnectRequest) -> Result<(), SessionError> {
        if request.proxy != ProxyConfig::Direct || !request.jumps.is_empty() {
            return Err(failure(
                ConnectionFailureKind::Unsupported,
                "a direct route",
            ));
        }
        if request.target != self.target {
            return Err(failure(
                ConnectionFailureKind::Denied,
                "the certified target",
            ));
        }
        Ok(())
    }
}

pub(crate) async fn establish(
    login: CertificateLogin,
    request: &ConnectRequest,
    driver: &SessionDriver,
    observed: &Arc<Mutex<Observed>>,
) -> Result<Handle<Client>, SessionError> {
    login.check(request)?;
    let stream = proxy::connect(&request.target, &ProxyConfig::Direct, None)
        .await
        .map_err(|error| error.into_session(&request.target.host))?;
    let stream = SourceStream::new(stream, driver.clone());
    let CertificateLogin {
        target,
        pinned_host,
        key,
        certificate,
    } = login;
    let mut handle = login::handshake_pinned(
        stream,
        &target,
        pinned_host,
        request.keepalive_interval,
        observed,
    )
    .await?;
    let result = handle
        .authenticate_openssh_cert(target.user.clone(), key, certificate)
        .await
        .map_err(crate::proof::native_error)?;
    if !result.success() {
        return Err(SessionError::AuthenticationFailed {
            user: target.user,
            host: target.host,
        });
    }
    Ok(handle)
}

fn failure(kind: ConnectionFailureKind, needs: &'static str) -> SessionError {
    #[derive(Debug, thiserror::Error)]
    #[error("a certificate login needs {0}")]
    struct Needs(&'static str);
    ConnectionFailure::new(kind, Needs(needs)).into()
}
